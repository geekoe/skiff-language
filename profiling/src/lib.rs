//! Rust profile 采样端（`rust.profile` 契约第 1 节）。
//!
//! 跨仓库契约见 `skiff-telemetry` 仓库 `doc/rust-profile.md`。本 crate 只实现
//! 采样端：后台线程按壁钟分钟边界对齐的窗口循环，用 pprof-rs 的
//! `ITIMER_PROF`（SIGPROF，天然只采运行态）做采样，每窗口产出折叠栈
//! （根→叶子用 `;` 连接）、按线程归因的样本与进程 CPU 时间，供 runtime /
//! router 进程内的后台任务消费（`take_window` 取窗口 → 发 PlatformEvent）。
//!
//! 函数级归因：解释器（或任何热路径代码）在 statement 边界调用
//! [`record_function_units`] 累加进程级全局计数表（FNV-1a 哈希分片的
//! `OnceLock` 单例，无锁优先、一次 lock 完成累加）；采样线程在窗口结束时
//! 读快照、diff 出窗口增量，按 units 比例分摊窗口 CPU 时间后放入
//! `ProfileWindow.functions`（units 降序，条数与 stacks 同限）。未启用采样
//! 或无计数时该字段为空。
//!
//! 依赖仅 `pprof` / `libc` / `anyhow`，不引入 frame-pointer 编译要求（pprof
//! 默认用 unwind backtrace）。CPU 时间读取按平台分支：Linux 解析
//! `/proc/self/stat` 的 utime+stime（clock ticks），macOS 用
//! `getrusage(RUSAGE_SELF)`；其余平台返回 0。
//!
//! 注意：pprof-rs 的 profiler 是进程级单例（全局 SIGPROF handler），同一进程
//! 内同一时间只允许一个采样窗口；本 crate 的窗口串行执行，天然满足。

use std::collections::hash_map::Entry;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
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

/// 单个函数的归因样本（Skiff 函数级 statement 计数）。
pub struct FunctionSample {
    /// 函数名（解释器侧传入，形如 `<module>;<symbol>`）。
    pub name: String,
    /// 窗口内该函数执行的 statement 计数（解释器每语句 +1）。
    pub units: u64,
    /// 按 units 比例分摊的窗口 CPU 时间（ms，`cpu_ms * units / total_units`）。
    pub cpu_ms: u64,
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
    /// 函数级归因，按 units 降序，条数截断到 `max_stacks`；未启用函数计数
    /// 或窗口内无计数时为空数组。
    pub functions: Vec<FunctionSample>,
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

// ---------------------------------------------------------------------------
// 进程级函数计数表（Skiff 函数级归因）
// ---------------------------------------------------------------------------

/// 函数计数表的 shard 数：解释器热路径高频写入，256 个 `Mutex<HashMap>` 分片
/// 摊薄锁竞争；同一 name 经 FNV-1a 哈希后总是落入同一 shard 与同一 key。
const FUNCTION_SHARDS: usize = 256;

/// 单个函数的计数单元。
struct CounterCell {
    /// 函数名（首次写入时记录；哈希冲突合并时保留先写入者）。
    name: String,
    /// 累计 statement 计数（单调累加，进程生命周期内不回退）。
    units: u64,
}

/// 进程级全局函数计数表（`OnceLock` 单例，惰性初始化）。
///
/// key 为函数名的 FNV-1a 64 位哈希；不同 name 落入同一 key 的冲突概率低，
/// 容忍合并：units 相加、name 保留先写入者。
static FUNCTION_TABLE: OnceLock<[Mutex<HashMap<u64, CounterCell>>; FUNCTION_SHARDS]> = OnceLock::new();

/// 取全局计数表，首次调用时惰性初始化全部 shard。
fn function_table() -> &'static [Mutex<HashMap<u64, CounterCell>>; FUNCTION_SHARDS] {
    FUNCTION_TABLE.get_or_init(|| std::array::from_fn(|_| Mutex::new(HashMap::new())))
}

/// FNV-1a 64 位哈希：只用于分片与 diff 键，不承担安全用途。
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// 记录一次函数 statement 计数（解释器热路径，每语句调用一次）。
///
/// 热路径只做一次 FNV-1a 哈希与一次分片锁（`units == 0` 或空名直接返回，
/// 不触碰全局表）。计数跨窗口累积，窗口增量由 [`function_units_diff`] 计算。
pub fn record_function_units(name: &str, units: u64) {
    if units == 0 || name.is_empty() {
        return;
    }
    let key = fnv1a(name.as_bytes());
    let mut cells = function_table()[key as usize % FUNCTION_SHARDS]
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    match cells.entry(key) {
        Entry::Occupied(mut entry) => {
            entry.get_mut().units = entry.get().units.saturating_add(units);
        }
        Entry::Vacant(entry) => {
            entry.insert(CounterCell {
                name: name.to_owned(),
                units,
            });
        }
    }
}

/// 遍历全部 shard，返回 key → units 快照（供窗口 diff 的 before/after）。
pub fn function_units_snapshot() -> HashMap<u64, u64> {
    let mut snapshot = HashMap::new();
    for shard in function_table() {
        let cells = shard.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        for (key, cell) in cells.iter() {
            snapshot.insert(*key, cell.units);
        }
    }
    snapshot
}

/// 计算两个快照的差值（窗口增量），输出 `(name, delta)`。
///
/// 过滤 `delta == 0`（含 after 计数回退/键消失的情况，按 0 处理），按 delta
/// 降序（同值按 name 字典序稳定）；名字从全局表按 key 查询（diff 期间再读
/// 一次 shard，查不到时为空串）。
pub fn function_units_diff(
    before: &HashMap<u64, u64>,
    after: &HashMap<u64, u64>,
) -> Vec<(String, u64)> {
    let mut diffs = Vec::new();
    for (key, after_units) in after {
        let delta = after_units.saturating_sub(before.get(key).copied().unwrap_or(0));
        if delta == 0 {
            continue;
        }
        diffs.push((function_name(*key), delta));
    }
    diffs.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    diffs
}

/// 从全局表按 key 查函数名；key 不在表中时返回空串。
fn function_name(key: u64) -> String {
    let cells = function_table()[key as usize % FUNCTION_SHARDS]
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    cells
        .get(&key)
        .map(|cell| cell.name.clone())
        .unwrap_or_default()
}

/// 后台采样主循环：每轮「睡到下一个对齐窗口起点 → 起 ProfilerGuard 采样 →
/// 睡满窗口 → 停止、算 cpu/wall、构建窗口入队」。
///
/// 窗口起点从「首个对齐分钟边界」开始，之后每次按 `interval_ms` 向前推进，
/// 保证窗口连续覆盖每个分钟（不能从停止时刻重新对齐——停止时刻已越过起点，
/// 重新对齐会跳过整个分钟）。
fn sampling_loop(config: ProfileConfig, stop: Arc<AtomicBool>, queue: Arc<Mutex<VecDeque<ProfileWindow>>>) {
    let interval_ms = config.export_interval_ms;
    let mut next_start_ms = (unix_now_ms() / interval_ms + 1) * interval_ms;

    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }

        // 睡到下一个对齐的窗口起点（壁钟分钟边界）。
        let now_ms = unix_now_ms();
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

        // 窗口起点函数计数快照：guard 构建成功后才取，起 guard 失败的跳窗
        // 不会污染下一窗口的 before。
        let function_units_before = function_units_snapshot();

        // 睡满整个窗口。中途收到停止信号则丢弃这个不完整窗口。
        if !sleep_until_stop(interval_ms, &stop) {
            drop(guard);
            break;
        }
        let wall_ms = window_instant.elapsed().as_millis() as u64;
        let cpu_ms = process_cpu_ms().saturating_sub(cpu_start);

        // 窗口结束：读函数计数快照，diff 出窗口增量并归因 CPU（见
        // [`collect_functions`]）。
        let function_units_after = function_units_snapshot();
        let functions = collect_functions(
            &function_units_before,
            &function_units_after,
            cpu_ms,
            config.max_stacks,
        );

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
                functions,
            });

        // 下一个窗口起点：从当前窗口起点推进一个窗口（不能从停止时刻
        // 重新对齐，否则会跳过整个窗口期）。
        next_start_ms += interval_ms;
    }
}

/// 把 pprof report 收敛成线程归因与折叠栈（按 samples 降序）。
///
/// 线程 CPU 时间按样本比例归因进程 CPU：`cpu_ms * 线程样本数 / 总样本数`。
/// 无样本时 threads 为空（契约允许）。折叠串按根→叶子用 `;` 连接，
/// 顺序与 pprof flamegraph 实现一致（frames 与内联符号均逆序）。
///
/// 噪声过滤：SIGPROF 可能在 libc 等待函数的用户态内部（condvar/kevent 的
/// syscall 往返间隙）触发，产生以等待原语开头的栈。这类样本不构成 CPU 时间
/// 归因（cpuMs 由内核计账给出），直接丢弃——见 [`is_wait_root`]。
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
        let mut folded = fold(frames);
        if is_wait_root(&folded) {
            continue;
        }
        total_samples += count;
        *thread_samples.entry(frames.thread_name_or_id()).or_insert(0) += count;

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

/// 把窗口内函数计数增量收敛成归因样本（见 [`function_units_diff`]）。
///
/// CPU 时间按 units 比例分摊窗口进程 CPU：`cpu_ms * units / total_units`
/// （与 threads 同法；无增量时函数为空）。diff 已按 units 降序，直接截断到
/// `max_functions` 条数上限（沿用 `max_stacks`，与 stacks 同限）。
fn collect_functions(
    before: &HashMap<u64, u64>,
    after: &HashMap<u64, u64>,
    cpu_ms: u64,
    max_functions: usize,
) -> Vec<FunctionSample> {
    let diffs = function_units_diff(before, after);
    let total_units: u64 = diffs.iter().map(|(_, units)| units).sum();
    let mut functions: Vec<FunctionSample> = diffs
        .into_iter()
        .map(|(name, units)| FunctionSample {
            name,
            units,
            cpu_ms: if total_units > 0 {
                cpu_ms.saturating_mul(units) / total_units
            } else {
                0
            },
        })
        .collect();
    functions.truncate(max_functions);
    functions
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

/// 已知等待原语集合：折叠栈以这些符号开头时，判定样本是 SIGPROF 在 libc
/// 等待内部触发的噪声（线程实际在等，不消耗 CPU），不参与归因。
const WAIT_ROOTS: &[&str] = &[
    "__pthread_cond_wait",
    "__psynch_cvwait",
    "__ulock_wait",
    "__semwait_signal",
    "kevent",
    "mach_msg2_trap",
    "semaphore_wait_trap",
    "__psynch_cvsignal",
];

/// 折叠栈首段是否为已知等待原语（`std::thread::sleep` 栈同样以等待开头，
/// 已被 `__psynch_cvwait`/`__semwait_signal` 覆盖）。
fn is_wait_root(folded: &str) -> bool {
    let Some(root) = folded.split(';').next() else {
        return false;
    };
    WAIT_ROOTS.contains(&root)
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
