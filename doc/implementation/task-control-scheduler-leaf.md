# Leaf Task: task-control scheduler（durable task dispatch 阶段 C2）

## 引用链

- 权威设计：`doc/architecture/durable-task-dispatch.md`（scheduler 只从
  TaskStore 的可见事实选择工作；负责 timing / fairness / capacity / claim /
  Runtime candidate selection，不解释业务 payload；claim 原子写
  state=leased + attempt generation 递增 + fresh AttemptId/lease id + lease
  owner/expiry + execution image witness；Runtime 必须在 lease 到期前 renew，
  settlement/heartbeat 携带当前 lease id；lease expiry 与 settlement 在
  store authority time 上 CAS 竞争；多 scheduler replica 并发扫描，正确性靠
  conditional claim；基础设施恢复用平台级 bounded backoff + jitter，不能
  hot retry；永久错误 → terminal platform-failed；本地 fast path 只是
  wake/claim 优化，不绕过 TaskStore 公平性）。
- 用户面契约：`doc/reference/dispatch.md`（本节点为 `TaskStatus` 语义提供
  调度事实，不定义新的用户面 surface）。
- 共享契约检查点 C1：`task-control` crate（TaskId/AttemptId/LeaseId/
  TaskState/TaskLease/TaskRecord/TaskStore trait + in-memory fake + Mongo
  adapter），已由 task_store 节点合入集成分支。
- 直接父节点：批次 `task-control-integration`，父文档
  `doc/implementation/task-control-batch.md`（集成 Agent
  `/root/task_control_integration`；本节点为 DAG 第 2 项 task_scheduler）。
- 仓库规则：`/Users/geek/workspace/AGENTS.md`、
  `/Users/geek/workspace/skiff/AGENTS.md`、
  `/Users/geek/workspace/multi-agent-development.md`。
- baseline：`e207db6206b7c384be11168c01da35b2f6a12c73`
  （`task-control-integration`，已 `git rev-parse` 验证；含 task_store 合并
  `71633eab` 与批次索引更新）。
- worktree：`/Users/geek/workspace/skiff-task-scheduler`，branch
  `task-scheduler`。

## 预检结论（只读，基线 e207db62）

- C1 契约已确认：`TaskStore::claim` 已原子写 state/attempt generation/
  fresh AttemptId/lease id/owner/expiry；`renew` 只接受当前 lease id 且
  expiry 未过；`settle` 与 `recover_expired_lease` 在 store authority time
  CAS 竞争；`scan_due` 按 `(state, due_at)` 可见性推进 scheduled→ready。
- C1 没有 scheduler 需要的三件事：store authority now 查询、按 lease expiry
  扫描可恢复任务、lease 有效期内由 admission 证明拒绝后的 release（回
  ready）。这三件都是 TaskStore 权威事实的读取/条件写，属于任务合同允许的
  trait 扩展，不改 C1 核心语义。
- `TaskRecord` 没有退避字段；按任务合同增加 scheduler 拥有的
  `retry_not_before: Option<DurableUtcTimestamp>`。
- 测试设施：`tests/support/{mod,contract,fixtures}.rs` 提供可控
  FakeClock/TestTime 与 record fixture；live-gate 惯例为
  `#[ignore]` + env var，不默认展开（本节点不新增 live gate）。
- 无兄弟节点重叠：task_store 已合并；本节点是批次第 2 项且唯一在途实现节点；
  router 尚未依赖 task-control（`router/Cargo.toml` 无 `skiff-task-control`），
  本节点不改 router。
- 共享主 worktree 当前有用户未提交改动
  `router/src/supervisor/actor_sink.rs`，与本任务无关，不触碰。

## 任务合同摘要

在 `task-control` crate 内新增 scheduler 模块，作为可插拔调度核心：

1. `Scheduler`：持有 TaskStore handle、admission seam、时钟/退避配置；
   提供 start/scan loop（due scanner：state=ready 且 due_at <= now，分批
   claim）、lease expiry recovery loop、wake fast path（立即任务 durable
   commit 后主动唤醒，不等扫描周期）。
2. admission seam：可插拔接口 `AttemptAdmission`，至少区分：
   - accepted：attempt 已开始执行；lease 由 scheduler 侧在 pending 期间续租
     （本节点选择：scheduler 循环内 renew 所有本 replica 已接受的 lease）；
   - rejected-provable：释放 claim（新 `TaskStore::release` CAS），task 回
     ready，带退避后可重新 claim；
   - uncertain：不 settlement、不 release，等 lease 过期走 store recovery →
     新 attempt（带退避）。
   另加 permanent-failure 分类：settle 为 terminal platform-failed。
3. 基础设施恢复与退避：`TaskRecord.retry_not_before`；
   `LeaseRecoveryInput` 增加 `retry_not_before`，recovery CAS 原子写入；
   新 `ReleaseInput` 同样原子写回退避；bounded backoff + jitter 防止
   hot retry。
4. 错误分类入口：`AdmissionDecision::PermanentFailure` → settle
   platform-failed；`RejectedProvable` / `Uncertain` 为暂时性，走退避后续
   attempt。
5. 测试：矩阵第 6、7、8、9、12、13、14 条的 scheduler 部分 + 多 replica
   并发（见自验收矩阵）。

## 设计决策（叶子执行合同内补充，不改权威语义）

1. **admission seam 形状**：`AttemptAdmission::admit(&TaskRecord) ->
   AdmissionDecision`。决策带 `reason` 供 telemetry；永久错误作为第四个
   decision（合同要求"至少"三类，增加永久类以满足错误分类入口）。
2. **renew 归属**：scheduler 侧续租。本节点采用最小实现：scheduler 维护本
   replica 已接受 attempt 的 `(TaskId, TaskLease)` 表，每个循环周期对全部
   活跃 lease 调 `renew`；`renew` 返回 Terminal/NotLeased/Stale/Expired/
   NotFound 时按 lease id 精确移除。不做每 attempt 独立 background task；
   配置要求 `lease_duration >= 2 * scan_interval`。真实 Router 接入
   （阶段 D2）可改为 per-attempt renew task 或 seam 自续租，不影响本契约。
3. **退避原子性（反事实删减）**：任务建议"增加字段 + 扩展对应 update"。
   本节点选择更小方案：没有独立的 `schedule_retry` 方法，而是把
   `retry_not_before` 放进 `recover_expired_lease` 与新增 `release` 的
   transition 输入，由 store CAS 原子写入（Mongo 用 `$max` 单调取后值）。
   反事实：若删去独立 update，所有需要退避的路径（lease 过期、不确定
   settlement、可证明拒绝）仍能原子设置退避；若保留独立 update，则存在
   "recovery 已回 ready、另一个 replica 的 due scanner 立即 claim"的
   hot-retry 窗口。独立 update 不覆盖任何必需能力，因此删减。
4. **authority now**：新增 `TaskStore::now()`（memory 取注入时钟；Mongo 用
   `isMaster.localTime` 服务端时间）。scheduler 用它计算 lease expiry 与
   retry_not_before，不把本地 wall clock 当作 durable 时间。
5. **expired-lease 扫描**：新增 `TaskStore::scan_expired_leases`，返回
   state=leased 且 lease expiry <= store now 的记录（按 expiry 升序、limit
   截断），供 recovery loop 发现所有 replica 遗留的过期 lease。
6. **release**：新增 `TaskStore::release`，CAS：leased && 当前 lease id &&
   lease 未过期 → ready + 清 lease + 原子设置 retry_not_before。已过期时
   recovery 竞争获胜；scheduler 不 release 过期 lease。
7. **capacity/fairness**：本节点最小实现为 `batch_limit` + `due_at` 升序 +
   conditional claim；`image_activatable` 为配置标志（阶段 C2 没有 activation
   registry，默认 true，D2 接入真实判断）。
8. **backoff 上界**：`delay = min(base * 2^(attempt_generation-1), max) +
   jitter`，jitter ∈ [0, jitter_span)，总上界 max + jitter_span - 1；
   jitter 用可注入 `Jitter` trait（生产 LCG，测试固定值），确保可测。
9. **wake fast path**：`watch` 计数器；`wake()` 单调递增，`run()` 在
   `watch.changed()` 与扫描周期之间 select。wake 只触发本地循环，不绕过
   TaskStore 的 claim/公平性。

## 写集（planned）

```text
doc/implementation/task-control-scheduler-leaf.md
task-control/src/model.rs                       # retry_not_before 字段
task-control/src/store.rs                       # now/release/scan_expired_leases；LeaseRecoveryInput 扩展
task-control/src/reducer.rs                     # release + recovery 退避原子写
task-control/src/memory.rs                      # 新方法实现
task-control/src/mongo.rs                       # 新方法实现 + DTO retryNotBefore
task-control/src/scheduler/mod.rs               # Scheduler + 循环
task-control/src/scheduler/admission.rs         # AttemptAdmission seam
task-control/src/scheduler/backoff.rs           # RetryBackoffPolicy + Jitter
task-control/tests/support/{mod,fixtures}.rs    # fixture 字段 + FakeAdmission/jitter harness
task-control/tests/support/contract.rs          # LeaseRecoveryInput 调用点补退避字段
task-control/tests/scheduler_memory.rs          # 聚焦 scheduler 测试
```

## 禁止

- 不改 runtime-transport wire、compiler、runtime、router 既有 `task.*` wire
  sink。
- 不改 task-control 已合并的 TaskStore 核心语义；既有 19 unit + 3 memory
  contract 测试必须保持编译与语义不变（新增字段/方法合法，recovery input
  调用点机械补字段）。
- 不改 `doc/reference/`、`doc/architecture/` 与 `doc/implementation/**`
  既有文件（本叶子文件为新增）。
- 不 push；不动共享主 worktree。

## 自验收矩阵

实际写集（commit 后与交接报告一致）：

```text
doc/implementation/task-control-scheduler-leaf.md
task-control/src/{lib,model,store,reducer,memory,mongo}.rs
task-control/src/scheduler/{mod,admission,backoff}.rs
task-control/tests/scheduler_memory.rs
task-control/tests/support/{mod,fixtures,contract,scheduler}.rs
task-control/tests/mongo_probe.rs
```

覆盖范围：scheduler 模块 + TaskStore 扩展（`now` / `release` /
`scan_expired_leases` / recovery 退避原子写）在 in-memory fake 与真实 Mongo
probe 上验证；矩阵第 6/7/8/9/12/13/14 条 scheduler 部分 + 多 replica 并发
完整覆盖；未跑完整 `pnpm verify`（按任务约定只做聚焦验证）。

自验收命令与结果：

```text
cargo test -p skiff-task-control        # PASS：25 unit + 3 memory contract + 9 scheduler（0 failed）
cargo check --workspace                 # PASS（仅 runtime-host 14 个预存 warning，非本写集文件）
SKIFF_TASK_CONTROL_MONGO_URL=... \
  SKIFF_TASK_CONTROL_MONGO_DB=skiff_task_control_c2_probe \
  cargo test -p skiff-task-control --test mongo_probe -- --ignored
                                        # PASS 13.84s（真实 rs0；含新增 scheduler store 扩展探针）
git diff --check                        # PASS
```

| 条款 | 代码证据 | 反向搜索证据 | 测试命令 |
| --- | --- | --- | --- |
| due scan：未来 task 到期前不可见、到期后可 claim；wall-clock 回拨不提前 | `src/scheduler/mod.rs` `scan_once`（scan_due + due_at 过滤 + conditional claim）；`tests/scheduler_memory.rs::due_scan_respects_due_at_and_clock_rollback`；store 语义由 C1 `scan_due`/claim CAS 保持 | `rg -n "retry_not_before" src/scheduler` 仅 scheduler 侧过滤；claim 未新增客户端时钟判定 | `cargo test -p skiff-task-control --test scheduler_memory due_scan` |
| 两个 scheduler 并发扫描同一批 task：每个 task 恰好一个成功 claim | `tests/scheduler_memory.rs::concurrent_replicas_claim_each_task_exactly_once`（32 task、两 replica `tokio::join!`、每 task generation=1、admitted TaskId 无重复） | `rg -n "AlreadyLeased" src/store.rs src/reducer.rs` 为唯一双 lease 防线；无 leader election 代码 | 同上 `concurrent_replicas` |
| lease expiry → recovery → 新 attempt（attempt generation 递增） | `recover_once` + `recover_expired_lease`（`src/store.rs`/`reducer.rs`）；`tests/scheduler_memory.rs::lease_expiry_recovery_creates_new_attempt_with_backoff`（gen 1→2） | `rg -n "attempt_generation" src/reducer.rs` claim 单调 +1 | 同上 `lease_expiry_recovery` |
| admission accepted：pending 期间 lease 续租，settlement 后 attempt 结束 | `handle_decision(Accepted)` 登记 active lease；`renew_active_leases` 按 lease id 续租/精确移除；`tests/scheduler_memory.rs::accepted_attempt_renews_until_settled`（expiry 从 +60s 推进到 +70s，settle 后 count=0） | `rg -n "remove_active_lease_if" src/scheduler/mod.rs` 只按 lease id 移除，防误删新 lease | 同上 `accepted_attempt_renews` |
| admission rejected-provable：claim 释放，task 回 ready | `AdmissionDecision::RejectedProvable` → `TaskStore::release`（CAS `leased+当前 lease+未过期` → ready，`$max` 原子退避）；`tests/scheduler_memory.rs::provable_rejection_releases_claim_with_backoff` | `rg -n "ReleaseOutcome" src` 无其它释放路径；release 未过期才成功，过期归 recovery | 同上 `provable_rejection` |
| admission uncertain：lease 过期 → recovery → 新 attempt 且受退避约束 | `AdmissionDecision::Uncertain` 不 settlement 不 release；`recover_once` 原子写 retry_not_before；`tests/scheduler_memory.rs::uncertain_admission_waits_for_expiry_then_backoff`（退避期 scan 不 claim，到期后 gen 2） | `rg -n "Uncertain" src/scheduler` 仅一个分支；`retry_not_before > now` 过滤在 `scan_once` | 同上 `uncertain_admission` |
| wake fast path：立即 task 提交后主动唤醒，不等扫描周期 | `watch` 计数器 `wake()` + `run()` select；`tests/scheduler_memory.rs::wake_fast_path_triggers_cycle_without_waiting_for_scan_interval`（scan_interval=1h，wake 后 2s 内 claim） | `rg -n "wake" src/scheduler/mod.rs` 仅本地通知，无 store bypass | 同上 `wake_fast_path` |
| 永久错误 → platform-failed terminal，不重试 | `AdmissionDecision::PermanentFailure` → `store.settle(PlatformFailed)`；`tests/scheduler_memory.rs::permanent_failure_converges_to_platform_failed_without_retry`（再次 scan 无新 admission，status=PlatformFailed） | `rg -n "PlatformFailed" src/scheduler src/store.rs` 收敛路径；无 max attempts 字段 | 同上 `permanent_failure` |
| 退避上界 + jitter 可测（fake 时钟） | `src/scheduler/backoff.rs`（`min(base*2^(gen-1), max) + jitter< span`）；`tests/scheduler_memory.rs::backoff_upper_bound_and_jitter_apply_end_to_end`（fake clock 断言 107/207/407/407 序列）+ `backoff::tests` | `rg -n "delay_millis" src/scheduler/backoff.rs` 唯一 delay 计算；`checked_add_millis` 溢出安全 | `cargo test -p skiff-task-control scheduler::backoff` + 同上 `backoff_upper_bound` |
| 多 replica 并发扫描 correctness（conditional claim，非 leader election） | 同上 concurrent 测试 + `Scheduler::run` 无选举/锁协调 | `rg -n "leader|election" task-control/src` 为空 | 同上 `concurrent_replicas` |
| 退避原子性（recovery/release 原子写，无独立 update 方法） | `reducer::merge_retry_not_before` + `recover_expired_lease`/`release`；Mongo `$max`；反事实见"设计决策 3" | `rg -n "schedule_retry" task-control` 为空（删减独立 update） | `cargo test -p skiff-task-control reducer::tests::release_returns_ready_with_monotonic_retry_not_before` |
| C1 既有测试不破坏 | 既有 19 unit + 3 memory contract 全部保留并通过；`LeaseRecoveryInput` 调用点仅机械补字段 | `git diff task-control/tests/support/contract.rs` 仅 recovery input 补 `retry_not_before: 0` | `cargo test -p skiff-task-control` |
| Mongo adapter 扩展在真实 store 上工作 | `src/mongo.rs`（`now`/`release`/`scan_expired_leases`/recovery `$max`/DTO `retryNotBefore`）；`tests/mongo_probe.rs::scheduler_store_extensions` | `rg -n '\$\$NOW' src/mongo.rs` 全部 authority 判定仍走服务端时钟 | 见上方 Mongo probe 命令（PASS） |
| 不改 wire / router task sink / 既有文档 / 不 push | 写集全部在 task-control/ 与新增叶子文档 | `git diff --name-only \| grep -E 'runtime/transport\|router/src/actor/task\|doc/reference\|doc/architecture'` 为空（exit 1） | `git diff --check` 通过 |
