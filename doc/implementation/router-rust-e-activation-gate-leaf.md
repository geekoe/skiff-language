# Router Rust Migration Batch 9 波 2 — E-activation Gate Leaf Task

日期：2026-08-03
状态：execution leaf（一次性有界会话；主 Agent 裁决生产缺口修复后继续）
Agent：`/root/dev_e_activation_gate`
集成目标：`/root/router_rust_integration_b9`

## 引用链

- 批次文档：`doc/implementation/router-rust-migration-batch-9.md`
  （E-activation 节点；baseline 集成 head `a9c8715b`）。
- 权威设计：`doc/implementation/router-rust-migration-plan.md`（draft v5），
  重点 §4.1（live transaction 步骤 1-10）、§4.2（cold recovery）、
  §7 E-activation（prepare/reject/commit/abort/stale ACK/cold recovery、
  CAS revision、retry 不重复 audit、decision 前 disconnect abort、
  decision 后 durable outcome reconcile）、§8
  `router-rust-activation-full-chain-live` / `router-live:activation-full-chain`
  （activate HTTP→durable prepare→real Runtime prepared→durable commit→
  epoch swap→Runtime commit→同 session re-register→new-generation HTTP
  request 成功，old captured epoch request 按原 lease 完成）。
- 兄弟 leaf：
  - `router-rust-migration-w-activation-leaf.md`（coordinator + ports；
    `SessionEnqueuePort` 生产接线归 E-activation）；
  - `router-rust-migration-w-activation-state-leaf.md`（Mongo repository /
    CAS / audit / retry）；
  - `router-rust-w-composition-leaf.md`（supervisor/components、
    `ActivationSessionEnqueuePort` / `ActivationTransactionSink` 装配）；
  - `router-rust-e-session-gate-leaf.md`（live harness 模式：真实 Router /
    Runtime binary + 临时 Mongo + relay 观测）；
  - `router-rust-e-gates-wiring-leaf.md`（波 1 生产接线与延迟 seam）。
- 冻结契约：C-activation-coordinator / C-router-activation-state /
  C-model-activation / C-process-lifecycle（control HTTP 路由归
  runtime/control listener，`/__skiff/activate-assembly` 为 canonical
  control 端点）。

## 主 Agent 裁决（2026-08-03，最小生产缺口修复授权）

只读预检发现 activate HTTP 入口在 Rust Router 侧缺失：`listener.rs`
RuntimeControl 分支对非 `/runtime` WS 请求一律空 200，`router/src` 无
`/__skiff/activate-assembly` 实现（仅 TS `assemblyControlPlane.ts` 有）。
修复必须动 listener + supervisor（超出原 gate 写面），已按停止条件上报。
主 Agent 授权按最小修复提案执行：

1. `router/src/activation/http.rs`（activation lane）：strict 解码
   `AssemblyActivationRequest`（artifact-model strict Deserialize + validate）
   → `start_live` → TS-parity JSON 响应/错误码；
2. `router/src/listener.rs`：RuntimeControl 分支增加
   `POST /__skiff/activate-assembly` 路由（body cap），additive；
3. `router/src/supervisor/mod.rs` + `start_runtime_control_listener` 签名：
   把 coordinator handle 传入 listener。

其余边界不变：禁止写其他 lane、runtime、deployment、router TS、
AGENTS.md、scripts README、verify selector graph、skiff-instance.mjs；
不操作 stable instance/Mongo/PM2/4004-4007；不跑全量 `pnpm verify`。

## 零 worktree 只读预检结论（锚定 a9c8715b）

1. `git rev-parse a9c8715b` =
   `a9c8715bb6829c31c9fa75a88e38dde8ccaee7f3`（integration head，波 1
   E-gates production wiring）；共享主 worktree 只读，预检全部用
   `git show`/`git grep` 完成，零写入。
2. W-activation coordinator 生产装配已就绪：`supervisor/mod.rs`
   `assemble_components` 注入 `ActivationSessionEnqueuePort`（sessions 端口）、
   `ActivationTransactionSink`（ACK 经 `coordinator.deliver_ack`）已装入
   `InboundSinkSet.activation_transaction`，coordinator 已进 consumer
   manifest。无缺口。
3. **activate HTTP 入口缺失（生产缺口）**：`listener.rs::handle_request`
   RuntimeControl 分支只处理 `/runtime` WS upgrade，其余路径
   `empty_response(200)`；`router/src` 对 `activate-assembly` 零命中；
   `http/server.rs:357` 在 public listener 对 control 路径显式 404 并注明
   “由 runtime/control listener 服务”，但该 listener 实际不服务。
4. Runtime 侧激活协议完整：`runtime/host/.../router_session/activation.rs`
   已实现 Prepare/Abort/commit 应答；`AssemblyActivationRequest` 在
   `skiff-artifact-model` 已有 strict Deserialize + validate。
5. session-live/bootstrap harness 模式已研读：`check-router-session-live.mjs`
   + `session_live_probe.rs`（真实 Router/Runtime binary、`ActivationStateMongoHarness`
   临时 replica set、45000-45999 端口租约、真实 compiler authoring、relay
   观测）；`verify-live-registry.mjs` 条目形态与
   `router-rust-integration.yml` append 模式已定位。

## 实现决策

### 1. `router/src/activation/http.rs`（activation lane，新增）

- `ASSEMBLY_ACTIVATION_CONTROL_PATH = "/__skiff/activate-assembly"`；
  body cap 1 MiB（TS `readBody` parity 常量）。
- `ActivationHttpHandler { coordinator, deadline }`：`handle(Request<Incoming>)`
  按帧读取 body（复用 `http/server.rs::read_request_body` 的 cap 模式，
  不复制生产管线），超限映射为 TS 分类结果；然后：
  1. method != POST → 405 + `allow: POST`；
  2. `serde_json::from_slice::<AssemblyActivationRequest>`（strict
     deny_unknown_fields + deserialize 时 validate）失败 → 400；
  3. `coordinator.start_live(request)` 同步失败（InvalidRequest → 400；
     TransactionInProgress → 409；MailboxFull/Shutdown → 503）；
  4. 等待 terminal phase（deadline 内）；`Committed` 且
     `health.activation_id == request.activation_id` → 200
     `{ ok, committed, activeAssembly, replicas }`（committed 由
     `CommittedActivation` DTO 序列化，activeAssembly 由请求候选构造；
     `replicas: []` 为 shape-parity seam，Rust composition 尚未暴露 replica
     snapshot）；
  5. 其余 terminal 按 TS `classifyActivationError` 词法映射
     （disconnected → 503 / timed out → 504 / invalid|must be|JSON → 400 /
     其余 → 409），错误体 `{ error: { code, message } }`（复用
     `HttpError::platform` + `json_body`）。
- 单元测试（`#[cfg(test)]`）：非 POST 405、malformed/unknown-field 400、
  body cap、成功链 200（真实 coordinator + fakes + ACK 驱动）、
  conflict/失败映射。

### 2. `router/src/listener.rs`（additive）

- `ListenerKind::RuntimeControl` 增加 `activation_http: Option<Arc<ActivationHttpHandler>>`；
- 新增 `start_runtime_control_listener_with_control(config, options,
  session_layer, activation_http)`；原 3 参 `start_runtime_control_listener`
  委托 `None`（既有调用/测试零变化）；
- `handle_request`：先保留 `/runtime` WS upgrade 分支，其次
  `path == ASSEMBLY_ACTIVATION_CONTROL_PATH` 且 handler 存在时交给 handler
  （非 POST 由 handler 返回 405）；无 handler 时保持原空 200。

### 3. `router/src/supervisor/mod.rs`（additive）

- `start_listeners` 构造 `ActivationHttpHandler::new(coordinator, deadline)`
  （deadline = `activation_prepare_timeout_ms * 2`，下限 30s），传入
  `start_runtime_control_listener_with_control`。
- supervisor 级测试（`router/tests/composition_supervisor.rs`）：真实 socket
  断言 control listener 对 `/__skiff/activate-assembly` 的 405/400/body-cap
  负例与既有路由（`/runtime` WS、其余空 200）零回归。

### 4. full-chain live harness（原 gate 交付）

- `scripts/check-router-activation-live.mjs`：仿
  `check-router-session-live.mjs`——真实 compiler package/assembly +
  config snapshot + actor-routing projection record、临时 Mongo replica set、
  45000-45999 租约端口、显式 `cargo build` router/runtime binary、注入 env
  驱动 ignored `router/tests/activation_full_chain_live_probe.rs`。
- 探针逐步证明：初始 committed 种子 → 启动真实 Router → 启动真实 Runtime
  （经 relay 观测或直连注册）→ `POST /__skiff/activate-assembly`
  （expected=N，候选 assembly/snapshot 已物化）→ durable prepare →
  Runtime 收到 Prepare 并回 Prepared → durable commit → epoch swap →
  Runtime 收到 Commit → 同 session re-register（new generation）→
  new-generation HTTP 请求成功；old captured epoch request 按原 lease 完成；
  decision 前 disconnect → durable abort；decision 后 disconnect → durable
  outcome reconcile（committed 保留）；cold recovery：committed 先发布、
  pending rebind、候选加载失败 durable abort；audit 不重复 / CAS revision /
  retry 由 Mongo repository 断言。
- `scripts/lib/activation_live_full_chain.mjs`（`activation_live_*` 前缀）：
  probe runner / 公共断言，复用 `activation-state-live-harness.mjs`。

### 5. registry / CI / tests / doc

- `scripts/lib/verify-live-registry.mjs`：追加自己的 key 块
  `router-rust-activation-full-chain-live`（selector `router-live:activation-full-chain`，
  id `live:router-rust-activation-full-chain`，MANAGED / live/manual，
  requiredExecutables `node/cargo/mongod/mongosh`，forbidUnchecked）。
- `scripts/tests/verify-live-registry.test.mjs`：仅 `LIVE_SELECTORS` 期望列表
  append `router-live:activation-full-chain` 的最小配套更新。
- `.github/workflows/router-rust-integration.yml`：append managed job
  `Router Rust Activation Full Chain (managed)` + classifier regex 扩展
  （`scripts/check-router-activation-live\.mjs` 与
  `router/tests/activation_full_chain_live_probe\.rs`）。
- `doc/implementation/router-rust-e-activation-gate-leaf.md`：本文件。

## 写集

生产（主 Agent 授权最小修复）：

- `router/src/activation/http.rs`（新）
- `router/src/activation/mod.rs`（仅 additive `pub mod http;` + re-export）
- `router/src/lib.rs`（仅 additive re-export）
- `router/src/listener.rs`（RuntimeControl 分支 + 4 参 listener 构造）
- `router/src/supervisor/mod.rs`（start_listeners 传 handler）

测试 / harness / tooling / doc：

- `router/src/activation/http.rs`（内联单元测试）
- `router/tests/composition_supervisor.rs`（control 路由 supervisor 级测试）
- `router/tests/activation_full_chain_live_probe.rs`（新，`#[ignore]`）
- `scripts/check-router-activation-live.mjs`（新）
- `scripts/lib/activation_live_full_chain.mjs`（新，`activation_live_*` 前缀）
- `scripts/lib/verify-live-registry.mjs`（仅本 gate 条目）
- `scripts/tests/verify-live-registry.test.mjs`（仅 `LIVE_SELECTORS` 行）
- `.github/workflows/router-rust-integration.yml`（仅 append 本 gate job）
- `doc/implementation/router-rust-e-activation-gate-leaf.md`（本文件）

禁止写：`router/src` 其余模块、`run_router`/`main.rs` 其它路径、runtime
crate、`runtime/transport/src`、deployment、router TS、AGENTS.md、
scripts README、verify selector graph、`skiff-instance.mjs`；不操作 stable
instance / Mongo / PM2 / 4004-4007；不跑全量 `pnpm verify`。

## 自验收矩阵

| 项 | 命令 / 证据 |
| --- | --- |
| activation http 单元 | `cargo test -p skiff-router activation::http`（405/400/body-cap/成功 200/失败映射） |
| supervisor 级路由 | `cargo test -p skiff-router --test composition_supervisor`（control 路由负例 + 既有路由零回归） |
| 全 crate 回归 | `cargo test -p skiff-router --no-fail-fast` 全绿（live probe 为 ignored） |
| full-chain live | `node scripts/check-router-activation-live.mjs`：prepare→commit→swap→commit→re-register→new request、old-epoch lease、decision 前 abort/decision 后 reconcile、cold recovery、audit/CAS/retry 全部通过且清理 |
| verify 注册表 | `node scripts/verify.mjs --only router-live:activation-full-chain --list` 含新条目 |
| workflow | `.github/workflows/router-rust-integration.yml` YAML 可解析；job 名稳定 |
| rustfmt/clippy | 触碰 Rust 文件 `cargo fmt --check`；`cargo clippy -p skiff-router --all-targets` 无新增 error |
| 写集干净 | `git status` 仅本叶子声明文件；未触碰禁止目录 |

## 交接

完成后提交到 `feat/router-rust-e-activation-gate`（不 push），直接向
`/root/router_rust_integration_b9` 报告 branch、worktree、commit/tree、
实际写集、自验收矩阵与已知 seam（`replicas: []` shape seam、abort 失败
消息为通用文案、control HTTP 负例分类与 TS 词法分类一致），并通知 root。

## 执行结果（提交前自验收填写）

（待填写）
