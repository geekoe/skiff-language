# Router Rust Migration Batch 7 — E-session Gate Leaf Task

日期：2026-08-02
状态：execution leaf（一次性有界开发会话）
Agent：`/root/dev_e_session_gate`
集成目标：`/root/router_rust_integration_b7`

> 已由 M4 取代（2026-08-06，runtime-lazy-deploy）：本文所述 activation 协调层 / epoch 部署机制
> （`assembly.activation` 帧族、`/__skiff/activate-assembly`、activation 状态仓库与配置项、
> committed/expected generation 术语等）已在 M4 全部下线，部署语义以
> [`doc/architecture/runtime-lazy-load-deployment.md`](../../architecture/runtime-lazy-load-deployment.md)
> 为准。本文保留为历史执行记录。

## 引用链

- 批次文档：`doc/implementation/router-rust-migration-batch-7.md`
  （E-session gate 节点；baseline `main@7d8779c4`）。
- 权威设计：`doc/implementation/router-rust-migration-plan.md`（draft v5），
  重点 §3.2/§3.4/§3.5/§3.6（session/directory/barrier/handshake）、
  §5.4（C-session/C-process-lifecycle）、§6.2(3)（TS→Rust→TS process/
  bootstrap/register/health）、§7 E-session（real handshake corpus、
  pre-auth limit/timeout、Register、ACK、health、barrier/reconnect、
  saturation 归零）、§8 `router-rust-session-live` /
  `router-live:session`（E-session slice 起 required managed CI；real
  Runtime bootstrap/register/reconnect/shutdown；无 unary）、§8 CI 条款。
- 兄弟交付（已合入 main@7d8779c4）：
  - E-bootstrap：`router/src/bootstrap/assembly.rs` +
    `router/src/listener.rs` 的 `run_router` 装配（committed epoch 发布后才
    bind listener；SessionLayer `attach_epoch_store`）、
    `router/tests/bootstrap_live_probe.rs`、
    `scripts/check-router-bootstrap-live.mjs`、
    `scripts/lib/verify-live-registry.mjs`（bootstrap 条目）、
    `.github/workflows/router-rust-integration.yml`（change classifier +
    managed bootstrap job）。
  - W-session：`router/src/session/` 的 handshake 状态机、
    `RuntimeRegistrationDirectory`（双索引/replacement/barrier）、
    `RuntimeRegistrationTransition`、pre-auth 上限与 deadline、consumer
    manifest + reserved terminal + ACK barrier + fail-stop、closed-family
    demux、`RuntimeHealthLedger`；真实 socket probe
    （`router/tests/session_handshake_probe.rs`）与预算/barrier probe。
  - contracts-session：`router-rust-migration-c-session-contract.md`、
    `c-model-registration-contract.md`、`c-process-lifecycle-contract.md`
    与 `runtime/transport/testdata/registration-handshake/` corpus。

冲突时以权威设计为准；本叶子只记录 E-session 装配与 live gate 的实现决策，
不改变冻结契约语义，不写生产 Router 代码。

## 零 worktree 只读预检结论（锚定 main@7d8779c4）

1. 基线：`git rev-parse main` = `7d8779c4b96c90c4d2d23748112ec1c0328091d7`；
   主 worktree 位于 integration 分支（仅多批次父文档，未合入 main）；
   兄弟 worktree `wt-w-actor` / `wt-w-websocket` 与本节点写集无重叠。
2. 生产装配已由 E-bootstrap 完成，本节点**不需要改任何 `router/src`**：
   `run_router` = `RouterBootstrapAssembly::assemble`（committed epoch）→
   `SessionLayer::attach_epoch_store` → `start_listeners_with_session`；
   shutdown = listener stop-accept（S1）→ `SessionLayer::shutdown`
   （S6 barrier，总 deadline 20s，超时 fail-stop 非零退出）→ assembly
   shutdown。真实 Router binary 可直接由 harness 构建并启动。
3. W-session 出口与真实 Runtime 握手逐帧对齐：Router 侧
   `router.bootstrap` → `runtime.capabilities` → `assembly.activation:Register`
   → `runtime.registered` ACK → `runtime.health` 观察；legacy register 与
   未实现 family 严格 terminal；replacement 先 cancel old 再 install new；
   close barrier 全 ACK 后才删除 exact session。
4. 真实 Rust Runtime 进程可独立启动：`cargo build -p runtime --bin runtime`；
   config `router: ws://127.0.0.1:<port>/runtime`、`runtime-home`、
   `environment`；driver 在 `run_forever` 内以 250ms→5s backoff 自动重连；
   replica id 由 `runtime-home/runtime-id` 决定（可预置固定值）；
   Runtime 收到 bootstrap 后经 `recover_durable_committed` 装载 committed
   assembly 再发送 capabilities + Register（tuple 与 bootstrap 完全一致）。
5. 可观测性缺口与方案：当前 Rust Router 的 runtime/control listener 对
   非 WS 请求只返回空 200，尚无 JSON `/__router/health` 投影（control
   endpoint 属后续 lane，C0-control 契约仍由 TS Router 提供；本节点禁止
   触碰 `router/src`，不补该投影）。因此 E-session 的 wire 断言通过
   **测试侧 WS relay** 完成：真实 Runtime 进程连接 relay，relay 连接真实
   Router binary，逐帧转发并记录双向 binary 帧；两端进程均为真实实现，
   relay 只是测试观测点。进程级 residue 由 Router/Runtime 退出码与端口
   关闭断言；`router-live:session` 不声称 unary/HTTP/WS 业务。
6. ingress 饱和可在真实 binary 上触发：进程级默认 inbound budget
   （64 帧 / 1 MiB，C-session §5.3）在注册后连续发送 >64 个
   `runtime.health` 帧即 abort exact session；outbound/mailbox 饱和与
   barrier/fail-stop 负例需要注入预算或 stuck consumer（生产常量无法在
   真实进程触发，且本节点禁止加生产 seam），由 W-session 既有
   `session_budget_probe.rs` / `session_consumer_barrier.rs` 单元级覆盖，
   叶子文档显式记录该边界。
7. `verify-live-registry.mjs` 的 `FIXED_COMMAND` script source 形态可直接
   追加 session 条目（key `router-rust-session-live` / selector
   `router-live:session` / id `live:router-rust-session`）；
   `verify-live-plan.mjs` / `verify-live-catalog.mjs` 无需改动；
   `scripts/tests/verify-live-registry.test.mjs` 只需在 `LIVE_SELECTORS`
   期望列表加一行（其余测试按索引操作，追加到 registry 末尾不破坏）。
8. CI workflow 已有 cheap change classifier（`pull_request` +
   `workflow_dispatch`，无 workflow 级 `paths`；非相关 PR 输出
   `related=false`，required job 显式成功）；本节点在其后追加 managed
   session job，并把 classifier regex 扩展为覆盖 session harness 与
   Runtime driver/host/loader 路径，保持“非相关 PR 显式成功”语义。
9. live harness 基建与 E-bootstrap 完全同构：临时 mongod replica set
   （45000-45999 租约）、真实 compiler authoring、真实 config snapshot、
   显式 `cargo build`；禁止端口集合 27017/4000-4007/44000-44999。
10. 任务可闭合；无需改公共契约、config schema、wire shape 或任何
    production 文件；不返回 `TASK_SCOPE_EXPANDED` / `TASK_NOT_EXECUTABLE`。

## 实现决策（在冻结契约语义内，全部为 test/harness/tooling 文件）

1. **live probe**（`router/tests/session_live_probe.rs`，`#[ignore]`，由
   harness 注入 env）：真实 Router 子进程（`CARGO_BIN_EXE_skiff-router`）
   + 真实 Runtime 子进程（env 传入 binary 路径）+ 测试内 WS relay。流程：
   种子 committed activation state（复用 E-bootstrap 的 repository
   seeding）→ 启动 Router → relay 监听租约端口 → 启动 Runtime（config 指向
   relay）→ 断言 relay 记录的双向帧序列：`router.bootstrap`（tuple 与
   committed 一致）→ `runtime.capabilities`（runtime_id == 预置 replica）
   → `assembly.activation:Register`（tuple + replica 一致）→
   `runtime.registered` ACK → 至少一帧 `runtime.health`。
2. **同一 replica 重连**：relay 支持按命令断开当前 pair（abort 两侧 pump）；
   Runtime 以同一 `runtime-home/runtime-id` 自动重连，relay 接受新连接并
   再次完成完整 handshake，断言 replica id 不变。
3. **replacement**：测试另开直连 WS（同 replica id）完成 handshake；断言
   old Runtime 连接被 Router 关闭、直连被注册；随后 Runtime 重连替换直连
   （直连被关闭、新连接重新注册）。relay 全程记录替换前后帧。
4. **pre-auth limit/timeout**：`runtime.maxConcurrency=4`；WS upgrade 后
   listener 的 pre-upgrade semaphore permit 已释放，registered Runtime 也
   不再占用 pre-auth 槽，因此 pre-auth pool（上限 = maxConcurrency）可容纳
   4 个直连 pre-auth 连接，第 5 个连接在 upgrade 后立即被
   `PreAuthLimitRejected` 关闭（无 bootstrap 帧）；释放后新连接重新收到
   bootstrap。bootstrap deadline 用进程级默认 10s：直连收到 bootstrap
   后不发 capabilities，断言 ~10s 内被关闭且无注册痕迹。
5. **ingress saturation**：直连以不同 replica 完成注册后连续发送 70 个
   `runtime.health` 帧（>64 帧上限），断言 exact session 被 abort；同时
   relay 上真实 Runtime 的 health 仍持续（只终止 exact session）。
6. **shutdown 归零 / fail-stop 边界**：SIGTERM Router → 断言退出码 0
   （barrier 全 ACK 成功；超时/fail-stop 会非零退出）、端口全部关闭、
   relay 观察到 session close；SIGINT Runtime（driver 安装 ctrl_c
   handler）→ 退出码 0。outbound/
   mailbox 饱和、barrier ACK 超时 fail-stop 保留 W-session 单元级覆盖，
   本 gate 不注入生产 seam。
7. **live harness**（`scripts/check-router-session-live.mjs`）：仿
   `check-router-bootstrap-live.mjs`：临时 source + 真实 compiler
   package/assembly authoring + config snapshot + actor-routing projection
   record；租约 3 个端口（http/runtime-control/relay，45000-45999）；
   `ActivationStateMongoHarness`；显式 `cargo build -p skiff-router
   --bin skiff-router` + `cargo build -p runtime --bin runtime`；运行
   ignored probe（注入 Mongo/artifact/environment/ports/runtime-bin/
   runtime-home env）；finally 清理 mongod/端口租约/临时目录并断言端口
   关闭。不触碰 stable instance / Mongo / PM2 / 4004-4007。
8. **verify live registry**：`scripts/lib/verify-live-registry.mjs` 追加
   `router-rust-session-live` 条目（script source +
   `FIXED_COMMAND`，`router-live:session`，`MANAGED` / `live/manual`，
   requiredExecutables `node/cargo/mongod/mongosh`，`forbidUnchecked: true`）。
9. **CI workflow**（`.github/workflows/router-rust-integration.yml`）：
   追加稳定 job 名 `Router Rust Session (managed)`，`needs:
   change-classifier` + `if: always()`，非相关时显式 skip-success，相关时
   安装 Node/Rust/MongoDB 并运行 `node scripts/check-router-session-live.mjs`；
   classifier regex 增加 `scripts/check-router-session-live\.mjs` 与
   `runtime/(driver|host|loader)/`。
10. **文档**：本叶子文件；leaf 交付记录在最后填写。

## 写集

- `router/tests/session_live_probe.rs`（新，`#[ignore]` live probe）；
- `scripts/check-router-session-live.mjs`（新 live harness，`check-*` 前缀）；
- `scripts/lib/verify-live-registry.mjs`（仅 session 条目）；
- `scripts/tests/verify-live-registry.test.mjs`（仅 `LIVE_SELECTORS` 期望
  列表加 `router-live:session` 的最小配套更新）；
- `.github/workflows/router-rust-integration.yml`（仅 session job +
  classifier regex 扩展）；
- `doc/implementation/router-rust-e-session-gate-leaf.md`（本文件）。

禁止写：`router/src`（含 `run_router`/listener）、runtime crate、
`runtime/transport/src`、deployment、AGENTS.md、scripts README、verify
selector graph、`skiff-instance.mjs`；不操作 stable instance / Mongo /
PM2 / 4004-4007；不跑全量 `pnpm verify`。若真实 roundtrip 暴露生产接线
缺口，停下报告 root（附精确失败证据与建议 owner），不顺手改生产代码。

## 自验收矩阵

| 项 | 命令 / 证据 |
| --- | --- |
| live 成功链 | `node scripts/check-router-session-live.mjs`：真实 Router + 真实 Runtime 经 relay 完成 bootstrap/capabilities/Register/ACK/health；同一 replica 重连与 replacement 通过 |
| pre-auth / timeout / saturation | 同 harness：pre-auth 上限拒绝与释放、bootstrap deadline 关闭、ingress 饱和只终止 exact session |
| shutdown 归零 | 同 harness：Router SIGTERM 退出 0 + 端口关闭；Runtime SIGTERM 退出 0 |
| verify 注册表 | `node scripts/verify.mjs --list` 含 `router-rust-session-live` / `router-live:session` |
| 聚焦 verify | `node scripts/verify.mjs --only router-rust,router-rust-process-smoke` 通过 |
| workflow | `.github/workflows/router-rust-integration.yml` YAML 可解析；session job 名稳定；无 workflow 级 `paths` |
| 写集干净 | `git status` 仅本叶子声明文件；`git diff main...HEAD` 聚焦；未触碰任何禁止目录 |

## 交接

完成后提交到 `feat/router-rust-e-session-gate`（不 push），直接向
`/root/router_rust_integration_b7` 报告 branch、worktree、commit/tree、
实际写集、自验收矩阵与已知 seam（relay 观测点、control health 投影属后续
lane、outbound/mailbox 饱和与 fail-stop 为 W-session 单元级覆盖），并通知
root（父 Agent）。

## 执行结果（提交前自验收填写）

（2026-08-02 提交前填写，全部通过）

1. live 成功链：`node scripts/check-router-session-live.mjs` 通过——真实
   `skiff-router` binary + 真实 `runtime` binary 经测试侧 WS relay 完成
   `router.bootstrap`（tuple == committed）→ `runtime.capabilities` →
   `assembly.activation:Register`（tuple + replica 一致）→
   `runtime.registered` ACK → 持续 `runtime.health`；同一 replica 断开重连
   （relay 断 pair → conn2 重新握手）；replacement（直连同 replica 客户端
   替换 Runtime session，Runtime 重连后替换直连客户端）全部通过。
2. pre-auth / timeout / saturation：maxConcurrency=4 时 4 个 pre-auth 槽
   占满后第 5 个连接 upgrade 后立即关闭（无 bootstrap 帧），释放后新连接
   重新收到 bootstrap；bootstrap deadline（默认 10s）关闭连接；直连
   replica 注册后连续 70 个 health 帧触发 inbound 64 帧上限，exact session
   被 abort，真实 Runtime 的 health 帧不受影响。
3. shutdown 归零：SIGTERM Router 退出码 0、http/runtime 端口全部关闭；
   SIGINT Runtime 退出码 0；relay 观察到 session close；无进程残留。
4. verify 注册表：`node scripts/verify.mjs --only router-live:session --list`
   展开 `live:router-rust-session`；`--help` 含
   `router-rust-session-live` / `router-live:session`；
   `scripts/tests/verify-live-registry.test.mjs` 的 registry contract
   `LIVE_SELECTORS` 期望已同步（20 项中 18 pass / 2 fail 为 worktree 未装
   `ws` module 的存量 loop-risk 环境条件，与本次条目无关，同 bootstrap
   gate 基线）。
5. 聚焦 verify：`node scripts/verify.mjs --only router-rust,
   router-rust-process-smoke` 2/2 passed（`router-rust:contracts` +
   `router-rust:process-smoke`）。
6. workflow：`.github/workflows/router-rust-integration.yml` 经 `yaml` 包
   解析通过；job 列表 `change-classifier` / `Router Rust Bootstrap
   (managed)` / `Router Rust Session (managed)`；session job `needs:
   change-classifier` + `if: always()`；classifier regex 含
   `scripts/check-router-session-live\.mjs` 与
   `runtime/(driver|host|loader)/`；无 workflow 级 `paths`。
7. 格式 / clippy：`cargo fmt -p skiff-router -- --check` 通过；
   `cargo clippy -p skiff-router --all-targets` 无 skiff-router 新
   warning/error（其余 crate 为既有 baseline warning）。
8. 写集：`git status` 仅本叶子声明文件；未触碰任何禁止目录（`router/src`
   / runtime crate / `runtime/transport/src` / deployment / AGENTS.md /
   scripts README / verify selector graph / `skiff-instance.mjs`）；
   `git diff main...HEAD` 聚焦。
9. 未发现生产接线缺口：真实 Router + 真实 Runtime roundtrip 一次打通，
   不需要停下上报 root；relay 仅测试观测点，无生产 seam。
