# Router Rust Migration Batch 9 波 2 — E-dispatch Gate Leaf Task

日期：2026-08-03
状态：execution leaf（一次性有界会话；root 已裁决扩展后继续）
Agent：`/root/dev_e_dispatch_gate`
集成目标：`/root/router_rust_integration_b9`

## 引用链

- 批次文档：`doc/implementation/router-rust-migration/execution/router-rust-migration-batch-9.md`
  （E-dispatch 行：`router-rust-dispatch-live` / `router-live:dispatch`；
  baseline `integration/router-rust-migration-batch-9@a9c8715b`，波 1 已合入）。
- 权威设计：`doc/implementation/router-rust-migration/execution/router-rust-migration-plan.md`（draft v5），
  重点 §3.3（capture → query → reserve → revalidate → enqueue → terminal）、
  §7 E-dispatch、§8 `router-rust-dispatch-live`、§9 CI。
- 冻结契约：`router-rust-migration-c-dispatch-contract.md`、
  `router-rust-migration-c-routing-query-contract.md`、
  `router-rust-migration-c-model-request-contract.md`、
  `router-rust-migration-c-session-contract.md`。
- 兄弟 leaf：`router-rust-migration-w-dispatch-leaf.md`、
  `router-rust-migration-w-routing-query-leaf.md`、
  `router-rust-w-composition-leaf.md`、`router-rust-e-gates-wiring-leaf.md`、
  `router-rust-e-session-gate-leaf.md`（harness/registry/workflow 模板）。

## root 裁决（2026-08-03）

预检发现真实 Rust Runtime 不宣告任何 dispatch capability，生产 dispatcher
的能力过滤（C-routing-query §3 规则 5）会把真实 Runtime 排除在 unary 候选
之外，E-dispatch 无法打到真实 Runtime。已上报 root，裁决为选项 A：

1. 小修授权（仅此一处 runtime 写）：`runtime/host/src/host/control_plane.rs`
   ——`queue_runtime_capabilities` 不再硬编码空 `dispatch_modes`，从已 admit
   assembly 的 HTTP gateway surface 派生 `unary`/`serverStream`（时序：
   `router.bootstrap` 时已 `recover_durable_committed`，之后才发
   capabilities）。同文件加单元测试（无 surface / 无 admit 保持空；有
   unary/serverStream surface 正确宣告）。禁止写 runtime crate 其他文件。
2. 其余按原 gate 任务：harness（fake ingress → admission/pending → real
   Runtime 全链 + selector/deployment/entry 负例 + timeout/disconnect/竞态 +
   归零）、registry 条目、CI job append、叶子文档、真实运行证据。
3. 若 runtime 修复后仍无法收敛（例如 surface 数据在 runtime 侧不可得），
   停止返回 `TASK_SCOPE_EXPANDED` 附证据。

## 零 worktree 只读预检结论（锚定 a9c8715b）

1. 基线：`git rev-parse integration/router-rust-migration-batch-9` =
   `a9c8715bb6829c31c9fa75a88e38dde8ccaee7f3`（波 1 E-gates wiring 已合入；
   波 2 五个 gate 并行，本节点独占自己的写集）。
2. fake ingress 可用路径（test-dispatch 或等价 seam）：Rust 侧没有 TS 的
   `/__skiff/test-dispatch` control endpoint；等价 seam 是生产
   `DispatcherHttpPort`（`supervisor/http.rs`，C-dispatch §7.2 契约形状
   `DispatchRequest { header, payload_bytes, timeout, cancel_signal }`）。
   探针在进程内用真实 `RouterSupervisor`/`RouterComponents` 装配
   （真实 Mongo repository + 真实 bootstrap/epoch/session/dispatcher），直接
   驱动 `components.http_dispatcher`——HTTP socket/selector 层由测试构造的
   `DispatchRequest` 替代（fake ingress），admission/pending/terminal 全走
   生产链，runtime 侧是真实 `runtime` 二进制进程（经 test-only WS relay）。
   这与 E-http 的 real HTTP socket 范围不重叠。
3. session-live harness 模式（模板）：
   `scripts/check-router-session-live.mjs` + `router/tests/session_live_probe.rs`
   （ignored test）：真实 compiler artifact（package/assembly/config snapshot）
   + 临时 Mongo replica set + 45000-45999 端口租约 + 显式 Rust binary 构建 +
   relay（观测帧序列）→ 进程级断言 → 清理。E-dispatch 复用同一结构。
4. registry/workflow 结构：`scripts/lib/verify-live-registry.mjs` 每 gate 一个
   key 块（`router-rust-session-live` 形态，managed/fixed-command/id）；
   `scripts/tests/verify-live-registry.test.mjs` 的 `LIVE_SELECTORS` 数组
   +1；`.github/workflows/router-rust-integration.yml` 每 gate append 一个
   managed job（needs change-classifier + if always + skip unrelated），
   classifier regex 追加本 harness 路径。
5. 生产缺口（已裁决）：`runtime/host/src/host/control_plane.rs:114-128`
   `queue_runtime_capabilities` 硬编码 `dispatch_modes: []`；session task
   （`router/src/session/task.rs:275-285`）据此记录 unary=false；
   `RuntimeCandidateQuery` 规则 5 排除 → dispatcher NoCandidate。修复点：
   `queue_runtime_capabilities` 经 `RuntimeHost::active_runtime_assembly()`
   （`loader/assembly_admission.rs:931`）取 `ActiveAssembly`，其
   `candidate().gateway_entries()`（`runtime/linker/src/assembly/candidate.rs:196`）
   提供 `LinkedGatewayEntry::protocol_surface()`（`gateway.rs` public），
   HTTP surface 的 `dispatch_mode`（unary/serverStream）可直接派生。
6. 编译 authoring 配方：package root 需 `package.yml`、`api.yml`、
   `service.yml`（`id: <service>`）、`http.yml`（typedJson unary 条目 +
   慢速条目，handler 参数 source `{ kind: http.body }`）、`main.skiff`
   （handler + `std.time.sleep`）。`skiff package build` 输出
   `serviceDeploymentReceipt.deployment`（exact ServiceDeploymentRef），
   `skiff assembly build --root-deployment '<ref>'` 把 deployment/ingress
   纳入 assembly（`compiler/driver/authoring.rs`）。
7. 写边界：可写 `runtime/host/src/host/control_plane.rs`（唯一 runtime
   文件，root 授权）、`scripts/check-router-dispatch-live.mjs`、
   `scripts/lib/verify-live-registry.mjs`（仅本条目行段）、
   `scripts/tests/verify-live-registry.test.mjs`（+1 selector 行）、
   `.github/workflows/router-rust-integration.yml`（append job + classifier
   regex）、`router/tests/dispatch_live_probe.rs`、本叶子文档。禁止写
   router/src、runtime crate 其他文件、runtime/transport/src、deployment、
   router TS、AGENTS.md、scripts README、verify selector graph、
   skiff-instance.mjs。

## 实现决策

### 1. runtime capabilities 派生（`runtime/host/src/host/control_plane.rs`）

- 纯函数 `dispatch_modes_from_gateway_entries(entries)`：遍历
  `GatewayEntryProtocolSurface`，HTTP surface 的 dispatch_mode 置位
  unary/serverStream；其他 protocol（websocketConnect/websocketJsonRpc）
  不影响。返回顺序固定 `[unary, serverStream]`。
- `queue_runtime_capabilities`：`active_runtime_assembly()` 为 None（无
  admit）→ 空；Some → 取 `candidate().gateway_entries()` 派生。
- 同文件 `#[cfg(test)] mod tests`：空 entries → `[]`；unary → `[unary]`；
  serverStream → `[serverStream]`；both → 固定顺序；WebSocket-only → `[]`；
  fresh host（无 admit）`queue_runtime_capabilities` 产出帧的
  `dispatch_modes` 为空。

### 2. E-dispatch live probe（`router/tests/dispatch_live_probe.rs`，ignored）

环境变量 `SKIFF_ROUTER_DISPATCH_LIVE_*`（mongo/db/artifact root/environment/
assembly identity/config snapshot/generation/control port/relay port/runtime
bin/runtime home/temp dir）。流程：

1. 种子 committed activation state（复用 session probe 的
   `EnvironmentActivationState::initial` + Mongo repository initialize）；
   materialize actor-routing projection。
2. 写 router config（lease 的 http/runtime port），
   `load_router_config` → `RouterComponents::assemble_with`（真实 Mongo
   repository）→ `RouterSupervisor::start_listeners`（真实生产装配）。
3. relay（纯转发 + 帧记录）+ 真实 runtime 二进制进程；等待并断言完整
   handshake，且 `runtime.capabilities.dispatch_modes == [unary]`（同时验证
   runtime 修复）。
4. fake ingress：`components.http_dispatcher.dispatch_unary(DispatchRequest{..})`
   ——header 从 `components.epoch`（assembly/deployment/gateway ingress/
   gateway entry identity）构造，canonical transport codec 编码，与
   `runtime/host/.../runtime_assembly_request/fixture.rs::canonical_header`
   同构。
5. 场景与断言：
   - 成功 unary roundtrip：typedJson handler 返回 payload，200 + body 精确；
     relay 观察到 `request.start` / `response.start` / `response.end`。
   - missing/invalid selector：deployment 不在 epoch → 503
     `ServiceUnavailable`；`mode=serverStream`（runtime 未宣告）→ 503。
   - wrong deployment/entry：deployment 在 epoch 但 gateway entry identity
     不匹配真实 entry → fail closed（runtime response.error 或 terminal），
     不遗留 pending。
   - duplicate request id：同一 request_id 第二次 submit → 409
     `DuplicateRequest`。
   - timeout：慢 handler + `DispatchRequest.timeout` 300ms →
     `HttpDispatchError::Timeout`，relay 观察到 `request.cancel`；等待后
     pending/permit 归零。
   - disconnect：慢 handler pending 期间 SIGKILL runtime 进程 →
     `RuntimeDisconnect` terminal，relay 无对应 `request.cancel`；
     permits 释放。
   - selection/replacement/disconnect 竞态：两个真实 runtime replica
     （A/B），A 满 4 个慢请求后第 5 个轮转到 B；SIGKILL A → 4 个
     RuntimeDisconnect；同 replica id 重启 A' 重新注册后新请求可路由；
     全部终态断言 `pending.unary == 0`、`permits_held == 0`、
     `releases == accepted`、`terminal.runtime_disconnect == 4`、per-session
     in-flight 零。
6. shutdown：`SupervisorListeners::shutdown` + `RouterSupervisor::shutdown` +
   SIGINT runtime，端口释放、无残留。

### 3. harness（`scripts/check-router-dispatch-live.mjs`）

仿 `check-router-session-live.mjs`：author 带 http.yml 的 service package →
`skiff package build` → `skiff assembly build --root-deployment` →
config snapshot → 临时 Mongo replica set → 租约 3 个端口（http/control/
relay）→ build `-p runtime --bin runtime`（router 以库方式在 probe 进程内，
不需要单独 build router binary）→ 运行 ignored
`dispatch_live_probe`（带全部 `SKIFF_ROUTER_DISPATCH_LIVE_*` env）→ PASS；
finally 清理 Mongo/port lease/temp。

### 4. registry / workflow / registry test

- `verify-live-registry.mjs`：key `router-rust-dispatch-live`，selector
  `router-live:dispatch`，id `live:router-rust-dispatch`，managed，
  requiredExecutables `['node','cargo','mongod','mongosh']`。
- workflow：append `router-rust-dispatch-managed` job（name `Router Rust
  Dispatch (managed)`，needs change-classifier，if always，skip unrelated），
  classifier regex 追加 `scripts/check-router-dispatch-live\.mjs`。
- registry test：`LIVE_SELECTORS` + `'router-live:dispatch'`，并断言新条目
  的 key/selector/id/ownership/tier。

## 写入边界

可写：

- `runtime/host/src/host/control_plane.rs`（root 授权唯一 runtime 文件）；
- `router/tests/dispatch_live_probe.rs`；
- `scripts/check-router-dispatch-live.mjs`；
- `scripts/lib/verify-live-registry.mjs`（仅 `router-rust-dispatch-live`
  条目行段）；
- `scripts/tests/verify-live-registry.test.mjs`（仅本 gate 相关行）；
- `.github/workflows/router-rust-integration.yml`（仅 append job + regex）；
- `doc/implementation/router-rust-migration/execution/router-rust-e-dispatch-gate-leaf.md`（本文件）。

禁止：`router/src`、runtime crate 其他文件、`runtime/transport/src`、
deployment、router TS、AGENTS.md、scripts README、verify selector graph、
`skiff-instance.mjs`、共享主 worktree、stable instance/Mongo/PM2/4004-4007；
不跑全量 verify。

## 自验收矩阵

| 项 | 命令 / 证据 |
| --- | --- |
| runtime capabilities 派生单测 | `cargo test -p skiff-runtime-host control_plane` 全绿（空/unary/serverStream/both/WebSocket-only/无 admit） |
| live 成功链 + 负例 + 归零 | `node scripts/check-router-dispatch-live.mjs` PASS（真实 Mongo + 真实 Router 装配 + 真实 Runtime：roundtrip、missing/invalid/wrong/duplicate、timeout、disconnect、replacement 竞态、pending/permit 归零） |
| registry 条目 | `node scripts/verify.mjs --only router-live:dispatch --list` 含 `live:router-rust-dispatch` |
| registry 测试 | `node --test scripts/tests/verify-live-registry.test.mjs` 本 gate 相关断言通过 |
| workflow YAML 解析 | `node -e "require('yaml').parse(...)"` 或等价解析通过；job 列表含 `Router Rust Dispatch (managed)` |
| 格式 / clippy（改动文件） | `cargo fmt -p skiff-runtime-host -p skiff-router -- --check` 相关文件通过；clippy 无新增 error |
| 写集干净 | `git status` 仅本叶子声明文件；`git diff a9c8715b...HEAD` 聚焦 |

## 交接

完成后提交到 `feat/router-rust-e-dispatch-gate`（不 push），直接向
`/root/router_rust_integration_b9` 报告 branch、worktree、commit/tree、实际
写集、自验收矩阵与已知限制（probe 在进程内装配 Router 库而非 binary；
runtime capability 修复点与理由），并通知 root。

## 执行结果（2026-08-03 提交前填写）

### 交付

1. runtime capabilities 派生（root 裁决 A）：
   `runtime/host/src/host/control_plane.rs`——`queue_runtime_capabilities`
   从 `active_runtime_assembly().candidate().gateway_entries()` 的 HTTP
   gateway surface 派生 `dispatch_modes`（unary/serverStream，固定顺序）；
   无 admit / 无 HTTP surface / WebSocket-only 保持空。同文件
   `#[cfg(test)] mod tests` 6 个单测（空、unary、serverStream、both 固定
   顺序、WebSocket-only、无 admit 帧为空）。
2. 生产缺口二（root 二次授权 supervisor 小修）：
   `router/src/supervisor/session_ports.rs` `DispatcherSessionConsumer`
   原来丢弃 `dispatcher.on_session_closed()` 返回的 terminals，HTTP phase
   只能等自身 deadline。修复：新增 `PendingHttpHandle`（deferred
   `Arc<PendingHttpRouter>`，与 SessionHandle 同形态），`on_session_closed`
   对每个 `PendingTerminal` 调 `router.deliver(request_id,
   HttpDispatchEvent::Terminal{terminal})`，deliver 失败忽略（permit 已
   释放，不 panic）；`router/src/supervisor/mod.rs` 装配同步（session
   构造前建 handle，`pending_http` 创建后 set）。shutdown barrier
   （C-process-lifecycle S6）走同一 consumer 路径受益。
3. E-dispatch live probe `router/tests/dispatch_live_probe.rs`（ignored）：
   进程内生产 `RouterSupervisor`/`RouterComponents`（真实 Mongo repository
   + 真实 committed epoch）+ 生产 listeners + 真实 `runtime` 二进制经
   test-only relay；fake ingress = 直接驱动生产 `DispatcherHttpPort`
   （契约 `DispatchRequest`）。场景全部通过：
   - 成功 unary roundtrip（200 + body，relay 观察到 request.start /
     response.end）；
   - missing/invalid selector（deployment 不在 epoch、serverStream 未宣告）
     → 503 ServiceUnavailable；
   - wrong deployment/entry → fail closed 无残留；
   - duplicate request id → 409 DuplicateRequest；
   - timeout → Timeout + `request.cancel` 帧 + 归零；
   - SIGKILL runtime disconnect → 立即 `runtime_disconnect` terminal（不等
     10s deadline）、无 cancel 帧、permit 释放；
   - selection/replacement/disconnect 竞态：两 replica 各 4 并发（容量不
     超 maxConcurrency=4）、queue-full 拒绝不泄漏、SIGKILL 一方恰好 4 个
     RuntimeDisconnect、同 replica 重启后新请求可路由；
   - 终态 `pending.unary == 0`、`permits_held == 0`、per-session 空、
     `releases == 15`（每个 accepted 恰好释放一次）、`runtime_disconnect`
     ≥ 5；
   - shutdown：listeners/supervisor 归零、control 端口关闭、runtime
     SIGINT 退出 0。
4. 单元测试 `router/tests/dispatch_consumer_terminal.rs`（4 个）：
   runtime disconnect 立即送 terminal（500ms 内，非 HTTP deadline）、
   多 pending 全送、HTTP phase 已消失不 panic 且 permit 恰好释放、
   shutdown barrier 路径同样立即送 terminal。
5. harness `scripts/check-router-dispatch-live.mjs`：std seed
   （`skiff-package-service-smoke-fixture --bootstrap-only`）→ 真实
   `skiff package build`（http.yml typedJson echo/slow）→ `skiff assembly
   build --root-deployment` → config snapshot（sources 匹配 deployment）→
   临时 Mongo replica set → 45000-45999 端口租约 → build `-p runtime` →
   ignored `dispatch_live_probe` → PASS；finally 清理。
6. registry / workflow / registry test：`router-rust-dispatch-live` /
   `router-live:dispatch` / `live:router-rust-dispatch`（managed，
   node/cargo/mongod/mongosh）；workflow append
   `router-rust-dispatch-managed` job + classifier regex
   `scripts/check-router-dispatch-live\.mjs`；registry test
   `LIVE_SELECTORS` +1 并断言新条目。

### 自验收

| 项 | 结果 |
| --- | --- |
| runtime capabilities 派生单测 | `cargo test -p skiff-runtime-host control_plane` 6 passed；`cargo test -p skiff-runtime-host --no-fail-fast` 全绿 |
| consumer terminal 单测 | `cargo test -p skiff-router --test dispatch_consumer_terminal` 4 passed |
| live 全链 | `CARGO_TARGET_DIR=<worktree>/target node scripts/check-router-dispatch-live.mjs` → `router-live:dispatch: PASS`（真实 Mongo + 生产 Router 装配 + 真实 Runtime；含负例、timeout、SIGKILL disconnect、replacement 竞态、pending/permit 归零） |
| skiff-router 全量回归 | `cargo test -p skiff-router --no-fail-fast` 69 个 test-result ok，0 failed |
| registry 条目 | `node scripts/verify.mjs --only router-live:dispatch --list` 含 `live:router-rust-dispatch` |
| registry 测试 | `node --test scripts/tests/verify-live-registry.test.mjs` 18 pass / 2 fail（存量 loop-risk ws module 环境条件，同 session gate 基线；dispatch 断言全过） |
| workflow YAML 解析 | `yaml` 包解析通过；jobs = change-classifier / Bootstrap / Session / Dispatch（name `Router Rust Dispatch (managed)`） |
| 格式 / clippy | `cargo fmt -p skiff-runtime-host -p skiff-router -- --check` 通过；clippy 本节点文件零 warning/error（其余为 baseline warning） |

### 写集

- `runtime/host/src/host/control_plane.rs`（root 授权唯一 runtime 文件）；
- `router/src/supervisor/session_ports.rs`（PendingHttpHandle +
  DispatcherSessionConsumer terminal 投递，root 授权）；
- `router/src/supervisor/mod.rs`（装配同步，root 授权）；
- `router/tests/dispatch_live_probe.rs`（新）；
- `router/tests/dispatch_consumer_terminal.rs`（新）；
- `scripts/check-router-dispatch-live.mjs`（新）；
- `scripts/lib/verify-live-registry.mjs`（仅本条目行段）；
- `scripts/tests/verify-live-registry.test.mjs`（仅本 gate 行）；
- `.github/workflows/router-rust-integration.yml`（仅 append job + regex）；
- `doc/implementation/router-rust-migration/execution/router-rust-e-dispatch-gate-leaf.md`（本文件）。

未触碰：runtime crate 其他文件、`runtime/transport/src`、deployment、
router TS、AGENTS.md、scripts README、verify selector graph、
`skiff-instance.mjs`、共享主 worktree、stable instance/Mongo/PM2/4004-4007。

### 已知限制

- 探针以进程内生产 Router 库装配（真实 `RouterSupervisor`/components +
  真实 Mongo + 真实 Runtime 进程），不另起 router binary；binary 进程级
  生命周期已由 session gate 覆盖。
- relay 是纯观测 pass-through（无帧篡改）；runtime capability 由 root
  授权的 runtime 修复真实宣告。
- 磁盘协调：并行 gate 构建共享磁盘，本 gate 只清理自己的 target 缓存；
  重型 cargo 前按 root 指示先查 df（< 3GB 等待或清理自己的 target）。
