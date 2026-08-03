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

## 主 Agent 裁决（2026-08-03，两轮最小生产缺口修复授权）

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

第二轮授权（cold recovery 生产装配，依据权威设计 §4.2）：

1. `router/src/bootstrap`（runner.rs/reader.rs/assembly）：durable pending
   不再整体 fail closed——committed 先构造并发布 active epoch，pending 安装
   recovery transaction；missing/malformed/identity mismatch 仍 fail
   closed；同步更新 E-bootstrap 时期测试（pending 用例改为 recovery 语义）。
2. supervisor 启动装配：committed 发布后调用
   `ActivationCoordinator::start_recovery`（不等待 participant，listener
   照常启动；expected replica 注册时由 recovery 流程发送 prepare）。
3. session 注册路径：additive `RegistrationObserver`，Runtime 注册 routable
   时通知 `ActivationCoordinator::register_recovery_session`（不改 session
   directory 内部语义）。
4. 增加 cold-recovery 单元/集成测试（committed+pending 双状态、rebind、
   候选加载失败 durable abort、进程退出后重启收敛）。

runtime crate 的 `dispatch_modes` 缺失由 root 指派 `/root/dev_e_dispatch_gate`
修复（已合入集成分支 fb60fb86）；本 gate 不写 runtime crate。

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
- `router/src/bootstrap/reader.rs`（`CommittedWithPending` 取代
  `FailClosedPending`；committed refs 仍先校验）
- `router/src/bootstrap/runner.rs`（`BootstrapRunOutcome { epoch, pending }`）
- `router/src/bootstrap/assembly.rs`（`pending_recovery()` accessor）
- `router/src/supervisor/mod.rs`（`start_recovery` 装配 +
  `SupervisorError::Recovery` + registration observer 接线）
- `router/src/session/observer.rs`（新 `RegistrationObserver` seam）
- `router/src/session/layer.rs` / `task.rs` / `mod.rs`（observer 注册与
  routable 通知 hook，additive）
- `router/src/activation/coordinator.rs`（`RegistrationObserver` impl）

测试 / harness / tooling / doc：

- `router/src/activation/http.rs`（内联单元测试）
- `router/tests/composition_supervisor.rs`（control 路由 supervisor 级测试）
- `router/tests/activation_recovery_wiring.rs`（新：recovery rebind、
  候选加载失败 durable abort、重启收敛）
- `router/tests/bootstrap_runner.rs` / `bootstrap_reader.rs` /
  `bootstrap_production_wiring.rs` / `bootstrap_live_probe.rs`（pending
  语义从 fail-closed 更新为 recovery）
- `router/tests/activation_full_chain_live_probe.rs`（新，`#[ignore]`）
- `scripts/check-router-activation-live.mjs`（新）
- `scripts/lib/activation_live_artifact.mjs`（新，`activation_live_*` 前缀）
- `scripts/lib/verify-live-registry.mjs`（仅本 gate 条目）
- `scripts/tests/verify-live-registry.test.mjs`（仅 `LIVE_SELECTORS` 行）
- `.github/workflows/router-rust-integration.yml`（仅 append 本 gate job）
- `doc/implementation/router-rust-e-activation-gate-leaf.md`（本文件）

禁止写：`router/src` 其余模块（http/ws/actor/dispatch/routing 等）、
`run_router`/`main.rs` 其它路径、runtime crate、`runtime/transport/src`、
deployment、router TS、AGENTS.md、scripts README、verify selector graph、
`skiff-instance.mjs`；不操作 stable instance / Mongo / PM2 / 4004-4007；
不跑全量 `pnpm verify`。

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
消息为通用文案、control HTTP 负例分类与 TS 词法分类一致、
`bootstrap-chain-corpus.json` 标签待 deployment owner 更新），并通知 root。
full-chain 真实运行证据在 E-dispatch runtime 修复合入集成分支后由 b9
协调复跑（本 worktree 基线 a9c8715b 不含 fb60fb86）。

## 执行结果（提交前自验收填写）

### 生产修复与单元/集成验证

1. activate HTTP 控制入口（第一轮授权）：`router/src/activation/http.rs`
   strict 解码 `AssemblyActivationRequest`（artifact-model strict
   Deserialize + validate）→ `start_live` → TS-parity JSON 响应/错误分类
   （`classifyActivationError` 词法一致：disconnected→503、timed out→504、
   invalid/must be/JSON→400、其余→409；405+allow、body cap 1 MiB、
   `{ok, committed, activeAssembly, replicas}`）。`listener.rs`
   RuntimeControl 分支路由 `POST /__skiff/activate-assembly`（无 handler 时
   保留旧空 200）；supervisor 把 coordinator handle 传入 listener。单元
   测试 8 项（405/malformed/unknown-field/schema 版本/body cap/concurrent
   409/decision 前 disconnect abort/full live 200/TS 分类规则）全绿；
   `composition_supervisor` 3 项全绿（control 路由负例 + 既有路由零回归）。
2. cold recovery 生产装配（第二轮授权）：reader `CommittedWithPending`
   （committed refs 先校验后发布）；runner/assembly
   `BootstrapRunOutcome` + `pending_recovery()`；supervisor pending 时
   `start_recovery`；session `RegistrationObserver` seam（routable 后通知
   coordinator rebind）。`activation_recovery_wiring` 3 项（rebind+commit、
   候选加载失败 durable abort、重启从 durable committed 收敛）；
   `bootstrap_runner` 7、`bootstrap_reader` 5、`bootstrap_production_wiring`
   7、lib 41、`activation_coordinator_unit` 13、corpus 1、repository
   contract 5、composition 系列全绿。
3. 真实 harness 回归（真实二进制 + 临时 Mongo + 真实 compiler artifact）：
   - `node scripts/check-router-session-live.mjs` PASS（session observer
     seam 无回归：bootstrap/capabilities/Register/ACK/health、重连、替换、
     pre-auth/timeout/saturation、shutdown 归零）；
   - `node scripts/check-router-bootstrap-live.mjs` PASS（pending 语义更新
     后：missing/malformed/identity mismatch 进程级 fail closed，pending
     进程启动并发布 committed epoch、干净退出）。
4. verify 注册表 / CI：
   - `node scripts/verify.mjs --only router-live:activation-full-chain --list`
     展开 `live:router-rust-activation-full-chain`；
   - `scripts/tests/verify-live-registry.test.mjs` 20 项中 18 pass / 2 fail
     （fail 为存量 loop-risk `ws` module 环境条件，与 E-session 基线一致；
     selector 声明断言 pass）；
   - `.github/workflows/router-rust-integration.yml` YAML 解析通过
     （PyYAML），job 列表含 `Router Rust Activation Full Chain (managed)`，
     `needs: change-classifier` + `if: always()`，无 workflow 级 `paths`。
5. 格式：触碰 Rust 文件 `rustfmt --edition 2021 --check` 通过；
   `cargo check -p skiff-router --all-targets` 零 warning。

### full-chain 真实运行证据（延迟项）

`check-router-activation-live.mjs` + `activation_full_chain_live_probe.rs`
已交付并通过编译检查；真实运行依赖 runtime dispatch_modes 修复
（E-dispatch 已合入集成分支 fb60fb86；本 worktree 基线 a9c8715b 不含）。
按 root 协调，合入顺序由 `/root/router_rust_integration_b9` 执行，合入后
在集成态复跑本 harness 产生 §8 全链证据（activate HTTP→prepare→real
Runtime prepared→commit→swap→commit→re-register→new-generation request、
old-epoch lease、decision 前 abort/decision 后 reconcile、cold recovery、
audit/CAS/retry）。

### 集成态复跑记录（2026-08-03，commit 010d1799）

并入集成分支 head 73a96a0f（merge c853b154）后复跑，逐层定位并修复：

1. **artifact variant identity**：deployment identity 剥离 human version
   label，仅改 package.yml version 产生相同 assembly（immutable record
   conflict）；三个 variant 改用互异 ingress path（/unary、/unary-new、
   /unary-third）+ variant-specific program。
2. **runtime-id**：probe 未 seed runtime-home/runtime-id，replica id 不匹配；
   已补 seed。
3. **service_db wire**：supervisor 把 `service_db_mongo_url: Some(...)`
   传给 coordinator → Prepare 携带 wire serviceDb → Runtime 拒绝
   （“use connection bootstrap”，TS 也不发）；改为 None。
4. **activation ACK sink 投递**：`sink_for` 排除 `Activation` family →
   Prepared/Reject 帧被 Unimplemented 终止 exact session，ACK 永远到不了
   coordinator；修复为可投递 activation_transaction sink（无 sink 时保持
   fail-closed）。该缺陷在 a9c8715b 基线已存在，属 E-activation 生产缺口。
5. **同 session re-register ACK 相位**：`HandshakeState::on_ack_written`
   只接受 RegisterValidated → post-commit 同连接 re-register 的 ACK 完成把
   session 打成 WrongOrder 终止；修复为 Registered 相位幂等接受。

复跑状态：激活链（activate HTTP→durable prepare→real Runtime
prepared→durable commit→epoch swap→Runtime commit→re-register→
new-generation request）已通过至 old-epoch request 断言；该断言仍 FAIL
（503 runtime_disconnect），根因在 **runtime 侧**：
`runtime/host/src/host/router_session/handshake.rs::on_registered` 只接受
RegisterSent 相位，post-commit 同连接 re-register 的第二个
`runtime.registered` ACK 触发 WrongOrder → Runtime 关闭 WS → Router
dispatcher 以 runtime_disconnect 终止在飞 old-epoch request。最小修复
（约 3 行，E-dispatch lane）：Registered 相位且 ack_runtime_id ==
expected_runtime_id 时幂等 Ok。已上报 root/b9，等待 E-dispatch 修复后
复跑取证。

本批 Router 侧修复聚焦测试全绿（session handshake/corpus/directory/demux、
composition、activation recovery/coordinator、bootstrap、lib 41）。

### 已知 seam / 协调项

- `replicas: []`：Rust composition 尚未暴露 replica snapshot 投影，
  control 响应保留 TS shape 空数组（shape-parity seam）。
- `deployment/tests/fixtures/bootstrap-chain-corpus.json` 的
  `pendingPresent.bootstrapOutcome` 仍为 legacy `failClosedPending` 标签
  （E-bootstrap 契约描述）；本 gate 的消费测试按新语义断言并注释，fixture
  标签更新归 deployment 侧 owner（集成时可一并机械更新）。
- abort 无关键词的失败消息按 TS 分类为 409（disconnect 具体原因未在
  coordinator health 中保留，语义与现有 corpus 一致）。
- `verify-live-registry.test.mjs` 的 2 项 loop-risk 环境失败为存量条件
  （worktree 无 `ws` module），与 E-session/E-bootstrap 基线一致。
