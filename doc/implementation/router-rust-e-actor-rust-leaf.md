# Router Rust Migration Batch 9 波 2 — E-actor-rust Leaf Task（router-live:actor gate）

日期：2026-08-03
状态：execution leaf（一次性有界会话）
Agent：`/root/dev_e_actor_gate`
集成目标：`/root/router_rust_integration_b9`

## 引用链

- 批次文档：`doc/implementation/router-rust-migration-batch-9.md`
  （波 2 E-actor-rust；baseline 集成 head `a9c8715b`）。
- 权威设计：`doc/implementation/router-rust-migration-plan.md`（draft v5），
  重点 §3.2（actor owner 表）、§3.3（catalog view）、§5.4/§5.5
  （C-spawn resolver / stateless `SpawnSubmitRouter` / sink bundle）、
  §7 E-actor-rust、§8 `router-live:actor`（two-replica actor chain）。
- 冻结契约：C-actor / C-model-actor / C-spawn / C-model-spawn / C-dispatch
  （§5 function-spawn correlation）/ C-session（§5.1 manifest、§6 demux）。
- 兄弟叶子：`router-rust-migration-w-actor-leaf.md`（六 owner）、
  `router-rust-w-composition-leaf.md`（composition 与延迟 seam 清单）、
  `router-rust-e-gates-wiring-leaf.md`（actor inbound sink 与 H3 spawn 缺口）、
  `router-rust-migration-m-spawn-repair-leaf.md`（spawn 方向修复 +
  `SpawnSubmitAcceptance` 数据面）、`router-rust-e-session-gate-leaf.md`
  （harness 先例）。

## 零 worktree 只读预检结论（锚定 a9c8715b）

1. baseline 锚定：`git rev-parse a9c8715b` =
   `a9c8715bb6829c31c9fa75a88e38dde8ccaee7f3`；worktree
   `/Users/geek/workspace/wt-e-actor-rust`（分支
   `feat/router-rust-e-actor-gate`）HEAD 即该 commit。
2. W-actor 六 owner 完整：catalog / ownership / activation broker /
   invocation relay / owner control / lease scheduler 均为 synchronous
   reducer；`ActorFrameSink`（supervisor/actor_sink.rs）已接 getOrCreate /
   find / remove / invoke / return / error / cancel / owner.control.ack /
   owner.failure；`actor.replace.request` 仍 fail closed
   （`ActorReplaceUnavailable`）。
3. spawn 现状（M-spawn-repair 后）：
   - `spawn.submit.request` = Runtime→Router；response/error =
     Router→Runtime（帧级 direction 表）；demux 已按帧级方向收窄，
     `InboundSinkSet.spawn = None`（收到 request 会终止 exact session）。
   - transport `SpawnSubmitAcceptance { request: SpawnSubmitRequestFrame
     (header+payload), spawn_id, request_id }` + `response_header()` 已就绪。
   - `RequestDispatcher::spawn_submit`（C-dispatch §5）无生产调用者；
     `SessionRuntimePeer::send_spawn_submit` 返回
     `Err("spawn.submit wire mapping is not wired until E-actor-rust")`。
   - `ActorLaneSpawnControl.submit_spawn` 只把
     `ActorMethodSpawnDispatch { spawn_request_id, caller_request_id,
     target }` 转发给 `RecordingActorMethodSpawnExecutionSink`（占位），
     丢失原始 wire header/payload。
4. lease/timer：`ActorLeaseExpiryScheduler`、relay/control/activation 的
   `expire_deadlines` 均无生产 tick；`mark_live/mark_active` 无人调用；
   `ActorSessionOwner` consumer 槽位已声明但未安装
   （session/consumer.rs `ConsumerKind::ActorSessionOwner`），runtime
   disconnect 不收敛 actor pending。
5. 两副本启动方式：TS 时代 `runActorFullChainAcceptance` 用
   `runInIsolatedTestRuntime({ runtimeReplicas: 2 })`（TS router）；Rust
   侧先例是 `check-router-session-live.mjs` + `session_live_probe.rs`
   （显式 Rust router/runtime binary + test-only WS relay + 临时 Mongo）。
   本叶子沿用 Rust 先例并扩展为两个独立 runtime-home + 两个 relay 副本。
6. 真实 artifact：`skiff-package-service-smoke-fixture` binary 可对
   `test-runner/fixtures/actor-full-chain-acceptance` 产出完整 receipt
   （assembly/configSnapshot/deployments/entrypoints），是既有真实
   compiler artifact 路径；A0 `records/actor-routing/current.json` 尚无
   production 生成器，由 harness 从 receipt/artifact 记录派生真实
   projection（schema v1）。

## 任务范围

1. 生产小修（仅 actor lane 装配面，含父节点明确授权的两处）：
   - `InboundSinkSet.spawn` 安装真实 spawn inbound sink；
   - spawn execution sink 装配（function 派生 wire 映射 +
     actor-method 真实执行 owner）；
   - lease/timer 全链：定时 sweep + `mark_live/mark_active` +
     renew（router 侧 registry）+ idle evict ACK 收敛；
   - `ActorSessionOwner` consumer 安装（runtime disconnect 归零）。
2. `scripts/check-router-actor-live.mjs` managed harness：真实 compiler
   artifact（actor-full-chain-acceptance fixture）+ 临时 Mongo + 显式
   Rust router binary + 两个独立 runtime-home 的真实 Runtime 进程
   （test-only relay），完整跑 get-or-create/claim/invocation/owner
   control/lease/function spawn/actor-method spawn，并做
   disconnect/replacement/concurrent claim/lease race/spawn mismatch
   fail-closed 与归零断言。
3. `scripts/lib/verify-live-registry.mjs` append 自己的 key 块
   （`router-rust-actor-live` / `router-live:actor`）；
   `scripts/tests/verify-live-registry.test.mjs` 同步 `LIVE_SELECTORS`
   与脚本存在性行。
4. `.github/workflows/router-rust-integration.yml` append 自己的 managed
   job（`Router Rust Actor (managed)`），沿用 change-classifier 模式。
5. 叶子文档（本文件）。

## 写入边界（worktree `/Users/geek/workspace/wt-e-actor-rust`）

可写：

- `router/src/actor/`（spawn sink / execution sink / wire store / timer
  pump 等 actor lane 小修）。
- `router/src/supervisor/actor.rs`（spawn execution sink 装配、timer
  task 装配、renew/mark_live 接线）。
- `router/src/supervisor/actor_sink.rs`（spawn 关联、idle-evict ACK、
  activation timeout 收敛、disconnect 帧侧归零）。
- `router/src/supervisor/session_ports.rs`（仅 `send_spawn_submit` wire
  映射 + `ActorSessionOwner` consumer 装配 + manifest 行）。
- `router/src/supervisor/mod.rs`（仅 `InboundSinkSet.spawn` 安装、
  consumer 列表/manifest 对应行）。
- `router/tests/actor_live_*`（新测试，若需要）。
- `scripts/check-router-actor-live.mjs`、`scripts/lib/actor_live_*`。
- `scripts/lib/verify-live-registry.mjs`（仅本 gate key 块）、
  `scripts/tests/verify-live-registry.test.mjs`（仅对应行）。
- `.github/workflows/router-rust-integration.yml`（仅 append 本 job）。
- 相关 doc（本文件）。

禁止：

- 其他 `router/src`（dispatch / session / ws / http / activation /
  bootstrap / listener / main 等）；runtime crate；`runtime/transport/src`；
  deployment；router TS；AGENTS.md；scripts README；verify selector
  graph；`skiff-instance.mjs`；共享主 worktree。
- 操作 stable instance / Mongo / PM2 / 4004-4007；不跑全量 verify。

## 实现决策（在生产代码前冻结）

### 1. Spawn inbound sink + wire store（router/src/actor/spawn_sink.rs 新）

- `SpawnWireStore`：`spawn_request_id -> SpawnSubmitRequestFrame` +
  admission outcome；sink 注册，execution sink 消费，响应写完后移除。
- `ActorSpawnFrameSink`（`InboundFrameSink`, family=Spawn）：解码
  `spawn.submit.request`（canonical codec），构造 `SpawnSubmit` DTO 调
  `RequestDispatcher::spawn_submit`（dispatcher 仍是 request/actor 双
  namespace 唯一裁决者，含 ambiguous），结果映射：
  - `AcceptedDerived` → `spawn.submit.response`（rpcId 回显、
    spawnId/requestId 来自派生结果）；
  - `ForwardedActorMethod` → 读 wire store outcome → response 或
    `spawn.submit.error`；
  - `Rejected` → `spawn.submit.error`（closed code 映射）；
  - 未找到 wire（孤儿 acceptance）→ fail closed 计数。
- 方向违规由 demux 已保证；`spawn.submit.response/error` inbound 不达
  sink。

### 2. Actor-method spawn 真实执行 owner（supervisor/actor_sink.rs）

- `ActorFrameSink` 增加 spawn invocation 关联表（owner+fence、无
  caller），`on_spawn_execution(wire, acceptance)`：registry current_owner
  → relay.invoke（spawn 专用 deadline/lease 常量）→ 编码
  `actor.owner.invoke`（args = spawn payload）→ 写 exact owner session；
  `actor.method.return/error` 走既有 settle 路径但不再转发（无 caller），
  relay pending 归零。
- `assemble_actor_components` 的 execution sink 改为可延迟安装
  （`DeferredSpawnExecutionSink` 转发到真实 sink），避免 ActorFrameSink
  与 ActorComponents 的构造环。

### 3. Function spawn wire 映射（supervisor/session_ports.rs）

- `SessionRuntimePeer::send_spawn_submit` 编码
  `RuntimeAssemblySpawnRequestStartFrameHeader`（mode unary、caller
  service、routing 来自 authority、invocation {kind: spawn,
  targetKind: function, target}、deadline 派生、trace 来自 wire store
  的 spawn header traceId + minted span），写 parent session；失败按
  dispatcher 既有 callback_error 语义。

### 4. Lease/timer 全链（router/src/actor/timer.rs 新 + supervisor/actor.rs）

- tokio pump（约 1s tick）：`lease.sweep`、`relay.expire_deadlines`、
  `control.expire_deadlines`、`activation.expire_deadlines`；outcome
  经 sink 写 owner cancel / caller error / waiter error 帧。
- invoke 路径 `mark_active` + `registry.renew`（router 侧 owner lease，
  与 TS parity：无 RenewLease wire 操作）；getOrCreate commit 后
  `mark_live`。
- IdleEvict ACK（accepted）→ `lease_scheduler.on_eviction_ack`；
  sink 维护 `eviction_request_id -> actor key` 映射。

### 5. ActorSessionOwner consumer（supervisor/session_ports.rs）

- `ConsumerKind::ActorSessionOwner` 安装：session close 时按 exact
  `replica_id#connection_generation` 调用 relay / control / activation
  disconnect + registry release（Disconnected），并触发 sink 帧侧归零。

### 6. Harness（scripts/check-router-actor-live.mjs + router/tests/actor_live_probe.rs）

- 临时 Mongo（45000-45999 租约，禁 4000-4007/44000-44999/27017）；
  fixture receipt 经 `skiff-package-service-smoke-fixture`；A0
  projection 从 receipt/package artifact 记录派生（真实 identities）；
  显式 `cargo build` Rust router/runtime；probe 内 spawn 真实 router +
  两个 relay + 两个真实 runtime（独立 runtime-home），HTTP 驱动 fixture
  probes，relay 断言 actor 帧序列，负例注入（伪造 spawn/actor.replace/
  断开/替换），终局 SIGTERM 优雅退出 + 端口关闭。

## 自验收矩阵

| 项 | 证据 |
| --- | --- |
| two-replica full-chain 真实运行 | `node scripts/check-router-actor-live.mjs` PASS（含 spawn/竞态/归零） |
| registry | `node scripts/verify.mjs --only router-live:actor --list` 含新条目；`scripts/tests/verify-live-registry.test.mjs` 全绿 |
| workflow YAML | `router-rust-integration.yml` 解析通过（simple-yaml / 语法校验） |
| 聚焦 Rust 测试 | `cargo test -p skiff-router --test actor_live_probe -- --ignored`（harness 驱动）+ 既有 actor/gates_wiring 回归 |
| 格式 / clippy | 触碰文件 rustfmt 通过；本叶子新增代码无 clippy error |
| 写集干净 | `git status` 仅本叶子声明文件；`git diff a9c8715b...HEAD` 聚焦 |

## 交接

完成后提交到 `feat/router-rust-e-actor-gate`（不 push），直接向
`/root/router_rust_integration_b9` 报告 branch、worktree、commit/tree、
实际写集、自验收矩阵与已知延迟 seam，并通知 root。

## 执行结果（2026-08-03 提交前填写）

### 主 Agent 裁决（2026-08-03）

per-session inbound frame 默认预算 64 → 4096（迁移驱动契约默认值修正，
`router/src/session/budget.rs`；仍 bounded fail-closed，不新增 config 键）；
同步更新 C-session 契约文档与依赖默认 64 的 E-session 饱和测试（改为显式
4200 帧超阈值，语义保持 fail-closed），其余 gate 饱和场景须使用显式预算。

### 交付

生产（actor lane 装配面 + 授权契约修正）：

- `router/src/actor/spawn_sink.rs`（新）：`SpawnWireStore`（原始 wire
  header/payload 关联 + 同步 admission outcome）。
- `router/src/supervisor/actor_sink.rs`：`ActorSpawnFrameSink`
  （`InboundSinkSet.spawn` 安装；request/actorInvocation 双 namespace
  parent authority、ambiguous/no-parent/target-kind mismatch fail
  closed、response/error 帧回写、wire 消费后移除）；真实 actor-method
  spawn 执行 owner（owner fence 校验/续租/owner.invoke 转发、spawn 专用
  deadline/lease、无 caller settle 归零）；invoke 路径 router 侧 lease
  renew + `mark_live`；IdleEvict ACK 关联；activation timeout 帧侧收敛；
  session close 帧侧归零。
- `router/src/supervisor/actor.rs`：`DeferredActorMethodSpawnExecutionSink`、
  `DispatcherSpawnParentLookup`（request parent 经 dispatcher pending）、
  `ActorIdleEvictControlPort::with_idle_evictions`、`ActorSessionOwnerConsumer`
  （`ConsumerKind::ActorSessionOwner` 安装）、timer pump（lease sweep +
  activation/control/relay deadline expiry）。
- `router/src/supervisor/session_ports.rs`：`send_spawn_submit` 编码真实
  `runtimeAssembly spawn request.start`（derived function spawn wire）。
- `router/src/supervisor/mod.rs`：spawn sink/execution/consumer/timer 装配行。
- `router/src/actor/lease.rs`：`forget`（release 后清理 scheduler 本地时钟）。
- `router/src/session/budget.rs`：`inbound_frames` 64 → 4096（root 授权）。
- `doc/implementation/router-rust-migration-c-session-contract.md`：默认值
  修正记录。

Harness / 注册 / CI / 测试：

- `scripts/check-router-actor-live.mjs`（新）：真实 compiler artifact
  （actor-live ordinary service，source 复制自 actor-full-chain-acceptance
  main.skiff）+ 临时 Mongo + 显式 Rust router/runtime binary + 两个独立
  runtime-home 的真实 Runtime（test-only relay），驱动 ignored
  `actor_live_probe`。
- `scripts/lib/actor_live_fixture.mjs`（新）：service source 编写 +
  compiler package/assembly/config-snapshot authoring + deployment record
  读取。
- `router/tests/actor_live_probe.rs`（新）：two-replica full-chain +
  负例/竞态/归零 + 优雅关闭。
- `router/tests/actor_live_lane.rs`（新）：wire store / lease forget /
  budget 默认单测。
- `scripts/lib/verify-live-registry.mjs`、`scripts/tests/verify-live-registry.test.mjs`：
  `router-rust-actor-live` / `router-live:actor` 条目 + selector 同步。
- `.github/workflows/router-rust-integration.yml`：`Router Rust Actor
  (managed)` job + classifier 路径。
- `router/tests/composition_supervisor.rs`：manifest/spawn sink 断言同步。
- `router/tests/session_live_probe.rs`：饱和测试改显式 4200 帧。

### 自验收

| 项 | 结果 |
| --- | --- |
| two-replica full-chain | `node scripts/check-router-actor-live.mjs` PASS（claim/activation/invocation/owner control/lease、actor-invocation spawn self/fanout/chain160、request-parent spawn ParentNotFound fail closed、spawn mismatch、direction violation/replacement、disconnect、frame-pair 归零、SIGTERM/SIGINT 优雅退出） |
| registry | `verify --only router-live:actor --list` 含 `live:router-rust-actor`；`verify-live-registry.test.mjs` 20/20 |
| workflow YAML | PyYAML 解析 `router-rust-integration.yml`，`router-rust-actor-managed` job 完整 |
| 聚焦 verify | `verify --only router-rust,router-rust-process-smoke` 2/2 |
| 全 crate 回归 | `cargo test -p skiff-router --no-fail-fast` 全绿（含 composition/session_budget/gates_wiring/actor_*） |
| session live 不回归 | `node scripts/check-router-session-live.mjs` PASS（4096 预算 + 显式饱和阈值） |
| 单测 | `actor_live_lane` 4/4；`actor_live_probe`（ignored，harness 驱动）PASS |
| 格式 / clippy | `cargo fmt -p skiff-router -- --check` 通过；本节点文件 clippy 零 warning |
| 写集 | `git status` 仅本叶子声明文件；`git diff a9c8715b...HEAD` 聚焦 |

### 已知延迟 seam / 阻断项（交接给集成与对应 gate）

1. real HTTP 入站 dispatch（Router→Runtime 业务请求）在本 baseline 被
   Runtime 能力帧的 `dispatchModes: []` 阻止（runtime capability seam，
   属 E-http lane/runtime）；本 gate 用 fake ingress（直接向真实 Runtime
   注入 canonical `request.start`）驱动 actor 全链，request-parent spawn
   以 ParentNotFound 负例证明 authority；positive request-parent spawn
   生命周期在 E-http/E-dispatch 合流后由 `actor_live_probe` 扩展。
2. `ActorActivationControlPort` ownerLeaseId mint 与 registry commit mint
   的 reconciliation 仍归 E-actor-parity（W-composition leaf 原记录）；
   当前 registry fence 与 control 帧 lease id 可不同，未影响本 gate。
3. catalog 只读 A0：live harness 物化 `skiff-actor-routing-projection-v1`
   （空 methods）并证明 Router 严格加载；typed query hit/miss 语义由
   W-actor 单测覆盖；真实 projection 生成器归 A1/A3 后续。
4. 磁盘纪律：本会话在 root 纪律消息前用 `cargo clean` 清理了两个已合入
   陈旧 worktree（skiff-actor-f2/f3）的 target 构建产物（约 25.7G，纯
   可重建 artifacts；源码未动）；此后未再清理任何其他 worktree。
