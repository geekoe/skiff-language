# Leaf Task: E3a durable task dispatch E2E 纵向链路验证（真实 consumer 路径）

## 引用链

- 权威设计：`doc/architecture/durable-task-dispatch.md`（完整阅读；Submission、
  Claim/Lease、At-Least-Once、Runtime Admission、Cancellation、Actor-method
  target、Observability 与 Retention 各节）。
- 用户面契约：`doc/reference/dispatch.md`（dispatch 语句/表达式、after/at、
  `std.task.TaskRef` / `std.task.status` / `std.task.cancel` 的逐字 kind
  拼写、TaskRef 可入 DB stored field 并跨 request 恢复）。
- 批次父节点：`doc/implementation/dispatch-e-batch.md`（集成 Agent
  `/root/dispatch_e_integration`；E3 e2e_observability 的拆分节点）。
- 已合并叶子：D1 `dispatch-d1-wire-leaf.md`、D2
  `dispatch-d2-router-leaf.md`、D3 `dispatch-d3-grammar-leaf.md`、D4
  `dispatch-d4-runtime-leaf.md`、E1 `dispatch-e1-std-task-leaf.md`、E2a
  `dispatch-e2a-actor-submit-leaf.md`、E2b
  `dispatch-e2b-actor-execute-leaf.md`。
- 仓库规则：`/Users/geek/workspace/AGENTS.md`、
  `/Users/geek/workspace/skiff/AGENTS.md`、
  `/Users/geek/workspace/multi-agent-development.md`。
- baseline：`2dd964023367fc830e2b7e9c51e7ceb1952ad65f`
  （`dispatch-e-integration` HEAD，已 `git rev-parse` 验证）。
- worktree：`/Users/geek/workspace/skiff-e3a-e2e`，branch `e2e-vertical`。
- 集成 Agent：`/root/dispatch_e_integration`；主 Agent：`/root`。本任务不
  merge、不 push、不写共享集成分支；共享主 worktree 只读。

## 任务合同摘要

实现阶段 E3a：durable task dispatch 的端到端纵向链路验证（真实 consumer
路径），作为“所有阶段做完”的完成性证据节点：

1. 测试服务 fixture：dispatch 语句与表达式、immediate 与 after/at 延迟、
   函数 target 与 actor-method target、`std.task.status` / `std.task.cancel`
   使用、TaskRef 存入 DB stored field 并跨 request 恢复。
2. 全链探针：source → compiler → artifact → runtime → router（worktree
   本地独立 instance，端口 4100–4102 风格，Mongo 27017；不使用/改动 stable
   instance 4000–4007，不注册 PM2）→ TaskStore durable create → scheduler
   claim → attempt 普通 request 执行 → settlement → `std.task.status` /
   `std.task.cancel`。覆盖：
   - 立即任务执行成功，status 收敛 `succeeded`；
   - 延迟任务到期前不执行（status `scheduled`/`ready`），到期后执行；到期前
     cancel → `canceled` 且不再执行；
   - cancel 与 claim 竞争（运行中 cancel → `alreadyStarted`）；
   - runtime 断连/重启 → lease 过期 recovery → 同一 TaskId 新 attempt
     （at-least-once，允许重复 effect）；
   - router 重启 → 已接受 task 不丢（Mongo 持久化）；
   - actor-method task 端到端执行（registry entry 存在路径；至少一条 snapshot
     恢复路径）；
   - TaskRef 跨 request 恢复后 status/cancel 可用。
3. 升级路径核对（见“升级路径核对结论”）。
4. 生产代码修复：E2E 需要的机械/局部修复（本叶子仅新增一个 RouterSupervisor
   装配 seam，见“关键实现决策”）。
5. 清理：探针用临时 instance/产物/测试库必须清理；构建缓存用完即清。

## 预检结论（只读，锚定 2dd96402）

- 集成分支 `dispatch-e-integration` HEAD = `2dd96402`（已 `git rev-parse`
  验证）；`main` = `033391ba` 是 merge-base，集成 worktree
  `/Users/geek/workspace/skiff-e-integration` 干净。
- 既有 live-gate 惯例：
  - `scripts/check-router-dispatch-live.mjs` + `router/tests/dispatch_live_probe.rs`：
    真实 compiler artifact + 临时 Mongo + 显式 Rust router/runtime binary +
    test-only WS relay + 生产 Router 组合/进程；
  - `scripts/check-router-actor-live.mjs` + `router/tests/actor_live_probe.rs`：
    真实 router/runtime 进程 + 两个 runtime-home + 真实 HTTP 驱动 + A0
    actor-routing projection 合成（`scripts/lib/actor-live-projection.mjs`）；
  - `scripts/lib/package-service-authoring.mjs`（`runCompilerAuthoring` /
    `runConfigSnapshotAuthoring`）、`scripts/lib/cargo-target-dir.mjs`
    （worktree 本地 `build/cargo-target`）、`scripts/lib/actor_live_fixture.mjs`
    （deployment record 读取）。
- 测试服务 fixture 结构：`test-runner/fixtures/*/{package.yml,service.yml,
  api.yml,http.yml,main.skiff}`；`http.yml` typedJson 入口把 body 作为 handler
  第一参数；DB 语法见 `doc/reference/db.md` 与
  `runtime/live-tests/internal/db_live.live.test.skiff`（`db object` /
  `db insert` / `db upsert` / `db find`）。
- task 控制面与 wire：
  - `runtime/transport/src/protocol/task.rs`：`task.submit.request/response/
    error`、`task.status.request/response/error`、`task.cancel.request/response/
    error`；
  - `router/src/task/{sink,control,admission}.rs`：TaskStore create /
    scheduler / claim / attempt admission / settlement；
  - `router/src/supervisor/mod.rs`：生产组合已装配 `MongoTaskStore`（数据库名
    `skiff-router`）与 scheduler worker；`RouterComponents::assemble_with_task_store`
    已存在（注入 repository + task store），`RouterSupervisor` 缺少同款装配
    seam（本叶子补上，见“关键实现决策”）。
- Mongo TaskStore 的 claim/scan 查询**没有 environment 过滤**：同一 Mongo
  URL + 同一数据库会跨 router 互相 claim。因此 E3a 探针在共享 Mongo 27017
  上使用**独立数据库**（不经由真实 router 二进制，而在 probe 进程内用生产
  `RouterSupervisor` 组合 + 注入的 `MongoTaskStore` / activation repository
  指向探针独有 DB），避免与 stable instance 的 `skiff-router` 数据库与
  scheduler 互相干扰。真实链路仅此一处“进程内组合”替代 router 进程，其余
  （真实 compiler artifact、真实 runtime 进程、真实 Mongo TaskStore、
  scheduler、HTTP listener、runtime WS）均为生产代码。
- 升级路径：`router/src/supervisor/actor_sink.rs::handle_replace` 仍 fail
  closed（`ActorReplaceUnavailable`，not wired until E-actor-rust）；
  `ActorOwnerControlBroker` 与 runtime 侧 upgrading state 已存在但 router 侧
  `actor.replace` 触发/等待 wiring 属于后续 E-actor upgrade 节点（主 Agent
  已裁决不在本节点实现）。
- E3b 无重叠：E3b 负责文档收敛（`doc/reference/` / `doc/architecture/` 既有
  文档）；本叶子只新增自己的叶子文件，不改既有文档。
- 磁盘：`/Users/geek/workspace` 约 19Gi avail；构建走 worktree 本地
  `build/cargo-target`，探针结束后清理临时目录与测试库。

## 关键实现决策（本叶子执行范围）

### 1. 测试服务 fixture（`test-runner/fixtures/durable-task-e2e-live/`）

真实 Skiff service source，HTTP typedJson 入口：

- `/submit-immediate`：`const task = dispatch markEffect(tag)`（表达式位置，
  TaskRef 入 `TaskEntry` DB stored field）；
- `/submit-after` / `/submit-at`：`dispatch ... after(3000ms)` /
  `dispatch ... at(futureInstant)`（本地 `type Instant = Date`，未来时间用
  `Date.fromEpochMilliseconds(Date.now().toEpochMilliseconds() + 3000)`）；
- `/submit-actor`：`const actor = std.actor.get<Counter>(tag)` 后
  `dispatch actor.increment()`（actor-method target，方法返回 void）；
- `/status` / `/cancel`：从 DB `TaskEntry` 恢复 TaskRef 后调用
  `std.task.status` / `std.task.cancel`，返回 `{ kind: string }`（union kind
  投影为普通 record，避免 typedJson 出口对 union 的 schema 要求）；
- `/effect`：读取 `Effect.visits`（函数 target 的持久 effect，`db upsert` +
  `visits += 1`），用于证明执行与重复 attempt；
- `/actor-count`：读取 actor 内存字段（证明 actor-method 执行）。

`TaskRef` 以 recoverable-envelope 存入 DB stored field；`status` / `cancel`
跨 request 从 DB 恢复后可用。

### 2. 探针（`router/tests/durable_task_e2e_live_probe.rs`，`#[ignore]`）

由 `scripts/check-durable-task-e2e-live.mjs` 驱动：

- harness 用真实 compiler 产出 package/assembly/config-snapshot artifact，
  合成真实 A0 actor-routing projection，在共享 Mongo 27017 上准备探针独有
  数据库，构建显式 `skiff-router` 与 `runtime` binary；
- probe 在进程内用生产 `RouterSupervisor` 组合（注入的
  `MongoActivationStateRepository` + `MongoTaskStore`，数据库名来自环境变量）
  启动真实 HTTP/control listener（4100/4101），test-only WS relay 监听
  4102，真实 runtime 进程经 relay 连 router；
- 真实 HTTP（raw TCP + `X-Skiff-Service` / `X-Skiff-Version` 头）驱动 fixture；
- 覆盖矩阵见“自验收矩阵”；
- router 重启用同一进程内 drop 旧组合 + 新连接/新组合（Mongo 持久化事实不
  变）模拟进程重启；runtime 重启用真实进程 kill/重新 spawn（同一 runtime-home
  → 同一 runtime-id）。

### 3. 生产代码修复（E2E 发现，主 Agent 已授权）

#### 3.1 TaskRef DB stored field 往返（实现 reference §3 已批准契约）

真实链路复现：`db insert TaskEntry { id task = <TaskRef> }` 在写侧成功，但
结果物化路径
`runtime/service-db/src/mapping.rs::business_value_from_recoverable_envelope_bson`
把信封解码后的 runtime value 用 untyped JSON 编码（TaskRef 是
`RuntimeValue::String(canonical)`，可正常渲染），随后
`runtime/eval/src/program_db.rs::decode_db_result` 用
`BoundaryUse::DbResultDecode` 的 plan-aware JSON 解码，而
`runtime/boundary/src/json_convert/{materialize,wire_decode,coerce}.rs` 对
`RuntimeTypeNode::TaskRef` 无条件拒绝（“opaque handle cannot cross the JSON
boundary”）。因此 TaskRef 实际无法从 DB stored field 恢复，与
`doc/reference/dispatch.md` §3 / `static-semantics.md` 的既有契约不符。

修复（主 Agent `/root` 授权，方案 1+2+3）：

- `runtime/boundary/src/json_convert/context.rs`：`StreamHandleScope` 与
  `HeapTraversalContext<Mode>` 增加 `allow_internal_task_ref` 标志；
- `runtime/boundary/src/json_convert.rs`：新增
  `InternalHandlePolicy { Refuse, AllowTaskRef }` 与
  `decode/encode/coerce_wire_plan_impl_with_handles`；既有入口默认
  `Refuse`；
- `runtime/boundary/src/json.rs`：`RuntimeBoundaryCodec` 按 use case 派生
  handle policy——`DbResultDecode` / `DbWriteProjection` 允许 TaskRef 以
  canonical string 往返，**其余 JSON 边界（HTTP/参数/返回/std.json）继续
  opaque 拒绝**；
- `runtime/boundary/src/json_convert/{materialize,wire_decode,coerce}.rs`：
  TaskRef 分支在允许时校验 canonical `skiff-task-v1:` 字符串；
- 测试：boundary `json_convert::tests::wire` 新增外部拒绝 + DB lane 往返
  2 例；service-db `tests::recoverable` 新增
  `recoverable_envelope_task_ref_field_roundtrips_canonical_string` 1 例。

说明：service-db mapping 读路径不需要 plan-aware 编码（untyped 编码对
TaskRef 的 canonical 字符串本来就正确）；主 Agent 授权文字中的“mapping 读
路径 plan-aware 编码”实测会破坏 `localType` 信封字段（Unknown plan），因此
按最小实现落点为 program_db 解码门控 + mapping 保持 untyped 编码。

#### 3.2 RouterSupervisor 装配 seam（机械小修）

`RouterComponents::assemble_with_task_store` 已存在但 `RouterSupervisor` 无
同款包装；新增 `RouterSupervisor::assemble_with_task_store(config, environment,
repository, task_store)`（镜像 `assemble_with`，~8 行），使 E2E 探针能在共享
Mongo 27017 上把 TaskStore/activation repository 注入探针独有数据库，避免
与 stable instance 的 `skiff-router` 数据库互相 claim。不改变任何行为/契约，
默认路径（`assemble` / `assemble_with`）不变。

### 4. 升级路径核对结论（主 Agent 已裁决）

- E2b 已把 forward/version 分支的终端映射落地：owner runtime
  `ActorUpgradingError` → task release/backoff；`ActorVersionRejectedError` /
  `ActorIncarnationReplacedError` → platform-failed；task attempt 与普通
  actor 调用共用 `ActorInvocationRelay` + `actor.owner.invoke` admission。
- 但 router 侧 `actor.replace.request` 仍 fail closed，普通 actor 升级路径
  （触发/等待）在 router 侧尚未存在；`doc/architecture/actor-model.md` 明确
  `replace`/`find`/`remove` 等注册控制操作不在第一版，v1 升级策略是 runtime
  侧 incarnation/epoch 机制。E2b 与 router-rust-e-actor-rust-leaf 均把 router
  侧 wiring 记录为后续 E-actor upgrade 节点。
- 主 Agent `/root` 裁决：**不实现 router 侧 actor.replace wiring，不扩展
  E-actor upgrade 范围**。E2E 覆盖分支 1（live 同 implementation）、2（entry
  冷激活）、3（snapshot 恢复）真实路径；分支 4/5 以 E2b 单测
  （`router/tests/task_actor_method_execution.rs`）作为代码级证据；v1 边界在
  本叶子记录。

## 禁止

- 不改 `doc/reference/` 与 `doc/architecture/` 既有文档；不改
  `doc/implementation/**` 既有文件（本叶子文件为新增）。
- 不动 stable instance（4000–4007）与常驻进程；不注册 PM2；不 push；不动
  共享主 worktree。
- 不实现 actor.replace 路由侧 wiring（主 Agent 裁决）；不扩展 E-actor
  upgrade 范围；不改 task-control / dispatch 语义。

## 实际写集（commit 后与交接报告一致）

```text
doc/implementation/dispatch-e3a-e2e-leaf.md                # 本叶子
router/src/supervisor/mod.rs                              # RouterSupervisor::assemble_with_task_store seam
router/tests/durable_task_e2e_live_probe.rs               # E2E 探针（ignored）
scripts/check-durable-task-e2e-live.mjs                   # managed harness
scripts/lib/durable_task_live_fixture.mjs                 # fixture authoring + 探针 env 准备
test-runner/fixtures/durable-task-e2e-live/               # 测试服务 fixture（5 文件）
scripts/lib/verify-live-registry.mjs                      # live gate 注册（本 gate key）
scripts/tests/verify-live-registry.test.mjs               # selector 同步
runtime/boundary/src/json_convert/context.rs              # internal task-ref 标志
runtime/boundary/src/json_convert.rs                      # InternalHandlePolicy + with_handles 入口
runtime/boundary/src/json_convert/{materialize,wire_decode,coerce}.rs  # TaskRef 门控分支
runtime/boundary/src/json.rs                              # codec use-case 派生 handle policy
runtime/boundary/src/json_convert/tests/wire.rs           # 外部拒绝 + DB lane 往返
runtime/service-db/src/tests/recoverable.rs               # TaskRef 信封往返测试
runtime/service-db/src/tests/recoverable_support.rs       # TaskRef metadata/expected 支持
```

## 自验收矩阵（提交后与交接报告一致）

| 设计/任务条款 | 代码证据 | 反向搜索证据 | 测试命令 |
| --- | --- | --- | --- |
| fixture：dispatch 语句/表达式 + after/at + 函数/actor target + status/cancel + TaskRef DB 恢复 | `test-runner/fixtures/durable-task-e2e-live/main.skiff` 真实 source；HTTP 入口 `/submit-*` `/status` `/cancel` `/effect` `/actor-count` | 编译期通过（真实 compiler）；探针断言逐条覆盖 | `node scripts/check-durable-task-e2e-live.mjs` |
| 立即任务 → succeeded；延迟任务到期前不执行、到期后执行；到期前 cancel → canceled 不执行 | 探针场景 1–2：HTTP 提交 + status 轮询 + effect 断言 | status/cancel 均经 DB TaskRef 跨 request 恢复 | 同上 |
| cancel 与 claim 竞争（running → alreadyStarted） | 探针场景 3：慢 target + 等 running 后 cancel | cancel 后 status 最终 succeeded（不改变状态） | 同上 |
| runtime 断连/重启 → lease 过期 recovery → 同 TaskId 新 attempt（effect 重复允许） | 探针场景 4：SIGKILL runtime；probe 直读 Mongo TaskRecord 断言 `attemptGeneration >= 2` 与 `Effect.visits >= 2` | 任务 TaskId 不变（DB `_id`）；settlement 未提前 | 同上 |
| router 重启 → 已接受 task 不丢（Mongo 持久化） | 探针场景 5：drop 组合 + 新组合 + 新 runtime，delayed task 到期后仍执行；status 恢复 | TaskRecord 在重启前已 durable create（probe 直读） | 同上 |
| actor-method task 端到端（分支 1 live / 分支 2 entry 冷激活 / 分支 3 snapshot 恢复） | 探针场景 6–8：提交 actor task、kill runtime/router、重启后执行；relay 断言 `activateInitial`；status succeeded + actor-count | 分支 4/5 代码证据 = E2b `task_actor_method_execution.rs`（10 例） | 同上 |
| TaskRef 跨 request 恢复后 status/cancel 可用 | 所有 status/cancel 请求均从 DB TaskEntry 恢复 TaskRef | fixture 无全局内存缓存 TaskRef | 同上 |
| TaskRef DB stored field 往返（生产修复） | `InternalHandlePolicy::AllowTaskRef` 仅作用于 `DbResultDecode`/`DbWriteProjection`；外部边界保持 opaque 拒绝 | boundary 测试断言 TypedJson 拒绝 + DB lane 往返；service-db 信封往返测试 | `cargo test -p skiff-runtime-boundary --lib`；`cargo test -p skiff-runtime-service-db --lib` |
| 生产代码 seam 不改变默认路径 | `RouterSupervisor::assemble_with_task_store` 镜像 `assemble_with` | `RouterSupervisor::assemble` / `assemble_with` 无行为 diff | `cargo check -p skiff-router` |
| 清理 | harness finally 删除临时目录；probe 结束 drop 探针独有 DB + 业务 DB；构建缓存探针后清理 | `git status` 仅本叶子声明文件 | 运行后检查 `mongosh` 无残留 DB、端口无残留监听 |
| 受影响 crate 聚焦回归 | router / runtime / compiler 相关测试 | 无回归 | 见验证记录 |

## 验证记录

### E2E 探针（真实纵向链路，`#[ignore]`，由 harness 驱动）

`node scripts/check-durable-task-e2e-live.mjs` → **PASS**。覆盖：

1. 立即函数 task：HTTP `/submit-immediate` → dispatch 表达式 → Mongo
   TaskStore durable create → scheduler claim → 真实 runtime 执行
   `markEffect`（DB effect）→ settlement `succeeded`；`std.task.status`
   经 DB 恢复 TaskRef 后收敛 `succeeded`；TaskRecord `state=succeeded`、
   `attemptGeneration=1`；relay 证据：`task.submit.request` →
   `task.submit.response` → task-attempt `request.start` → `response.end`。
2. `after(3000ms)`：到期前 status `scheduled`、effect 0；`/cancel` →
   `canceled` 且 4 秒后 effect 仍 0（无 attempt）；第二个延迟 task 到期后
   执行成功。
3. `cancel` vs `claim`：慢 target 进入 `running` 后 cancel →
   `alreadyStarted`，最终 `succeeded`。
4. runtime SIGKILL 断连 → scheduler 停止 renew → lease 过期 recovery（约
   60s）→ 同一 TaskId 新 attempt（`attemptGeneration=2`），effect 重复
   允许（`visits>=2`）。
5. router 组合 shutdown + 重装配（同一 Mongo DB）→ 已接受 delayed task
   到期后仍执行；TaskRef 跨 request 恢复可用。
6. actor-method task（分支 1 live）：actor 方法内 `dispatch self.increment()`
   + TaskRef 入 DB；`succeeded` + actor count=1 + `actor.owner.invoke` 帧。
7. actor-method task（分支 2）：kill runtime（router 保留 entry）→ 重启
   runtime → 冷激活（`actor.owner.control` activateInitial）→ `succeeded`。
8. actor-method task（分支 3）：kill runtime + router（registry entry 丢失）
   → 重启 → task snapshot 恢复最小 entry → 冷激活 → `succeeded`。
9. 清理：探针 DB（`skiff_e2e_task_live`、业务 DB）drop 后无残留；端口
   4100–4102 释放；临时目录删除。

### 聚焦回归

- `cargo test -p skiff-runtime-boundary --lib`：174/174 PASS（新增 TaskRef
  external-refusal + DB-lane roundtrip）。
- `cargo test -p skiff-runtime-service-db --lib`：145/145 PASS（新增
  recoverable-envelope TaskRef 往返）。
- `cargo test -p skiff-router --lib`：69/69 PASS。
- `cargo test -p skiff-router --test task_control_unit --test
  task_actor_method_execution --test dispatch_admission_corpus`：18 + 10 + 2
  PASS。
- `cargo check --tests -p skiff-router` PASS。

## E2E 发现并处理/记录的缺陷

1. **DB TaskRef stored-field 往返缺失**（已修复，见“关键实现决策 3.1”）：
   属于实现 reference §3 已批准契约，非设计变更；外部 JSON 边界保持 opaque
   拒绝。
2. **compiler record pattern 一律降级为 wildcard**（未修复，设计级）：`match`
   （如 `match (std.task.status(ref)) { { kind: "scheduled" } => ... }`）的
   record pattern 在 `compiler/lowering/src/function_lowering.rs`
   `lower_pattern_and_bind` 无条件返回 `PatternIr::Wildcard`，导致 union 上
   的 record 判别恒命中第一分支。已确认 artifact IR（8 个 wildcard arm）。
   修复需扩展 artifact `PatternIr` + runtime match 判别（跨契约，非 E3a
   机械修复范围）。E2E fixture 用 `std.json.encode<T>` → `decode<{kind}>`
   投影 kind 绕过；该缺陷已报告主 Agent `/root`，建议交语言批次修复。
3. **E2a actor-method 提交侧仅支持 actor execution frame**：普通 HTTP
   request 中 `dispatch actor.method()` 被 definite reject（
   `authenticated_actor_handle` 需要 actor frame）。这是 E2a 叶子记录的既有
   边界；fixture 改为 actor 方法内部 `dispatch self.increment()` 并把 TaskRef
   写入 DB（handler 传入 id）。
4. **探针侧数据库名推导错误**（探针 bug，已修复）：业务 DB 名必须与 runtime
   `service_storage_database_name` 一致（`.`→`~`、`/`→`~~`）；此前 drop
   打到不存在的库导致跨运行残留。

## 交接

完成后把 branch、worktree 路径、commit/tree、实际写集和自验收矩阵直接报告给
`/root/dispatch_e_integration`，并通知主 Agent `/root`。
