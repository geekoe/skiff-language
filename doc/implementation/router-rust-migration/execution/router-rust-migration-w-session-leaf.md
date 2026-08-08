# Router Rust Migration Batch 4 — W-session Leaf Task

日期：2026-08-02
状态：execution leaf（一次性有界开发会话）
Agent：`/root/dev_w_session`
集成目标：`/root/router_rust_integration_b4`

> 已由 M4 取代（2026-08-06，runtime-lazy-deploy）：本文所述 activation 协调层 / epoch 部署机制
> （`assembly.activation` 帧族、`/__skiff/activate-assembly`、activation 状态仓库与配置项、
> committed/expected generation 术语等）已在 M4 全部下线，部署语义以
> [`doc/architecture/runtime-lazy-load-deployment.md`](../../../architecture/runtime-lazy-load-deployment.md)
> 为准。本文保留为历史执行记录。

## 引用链

- 直接父批次：`doc/implementation/router-rust-migration/execution/router-rust-migration-batch-4.md`
  （W-session 节点；baseline `main@7683b7c8`）。
- 权威设计：`doc/implementation/router-rust-migration/execution/router-rust-migration-plan.md`（draft v5），
  重点 §3.2（owner/invariant、`RuntimeRegistrationDirectory` 双索引、
  replacement/cancel/barrier）、§3.4（identity/fence）、§3.5（真实 Runtime
  handshake）、§3.6（disconnect 是 cancellation + barrier）、§5.4
  （C-session 解锁 W-session）、§5.5（demux 与 sink bundle）、§6.1
  （未实现 family 终止 exact session）、§7（E-session）。
- 冻结契约：
  - `doc/implementation/router-rust-migration/contracts/router-rust-migration-c-session-contract.md`
  - `doc/implementation/router-rust-migration/contracts/router-rust-migration-c-model-registration-contract.md`
  - `doc/implementation/router-rust-migration/contracts/router-rust-migration-c-process-lifecycle-contract.md`
  - `doc/implementation/router-rust-migration/contracts/router-rust-migration-c-net-contract.md`（listener 机制）
- 同链契约/参考 corpus（test-only，W-session 消费同一 fixtures）：
  - `runtime/transport/testdata/registration-handshake/`（frames.json + 19 个场景）
  - `runtime/transport/tests/registration_handshake_corpus.rs`
  - `runtime/transport/tests/session_directory_contract.rs`

冲突时以权威设计为准；本叶子只记录 W-session 实现决策，不改变冻结契约语义。

## 零 worktree 只读预检结论（锚定 main@7683b7c8）

1. 基线：`git rev-parse main` = `7683b7c8007a374ae07cb62c7723ced62929100b`。
2. PR 0b listener/main 结构（`router/src/listener.rs`、`router/src/main.rs`）：
   `start_listeners` + `run_router` 装配 public + runtime/control 两个 listener；
   C-net 机制完整（hyper 1 with_upgrades、tokio-tungstenite 0.26、
   Semaphore、watch + drain + deadline abort）；`/runtime` WS upgrade 目前只
   drain 帧，无业务协议。
3. transport 现有 codec（canonical owner 保持不动，W-session 只消费）：
   - `assembly_activation.rs`：`AssemblyActivationControl::Register`
     （environment/generation/assembly/config_snapshot/replica_id）+
     `encode_assembly_activation_frame` / `decode_assembly_activation_frame`
     与方向校验；
   - `protocol/session.rs`：`RouterBootstrapFrameHeader` +
     `decode_router_bootstrap_frame_header`、`RuntimeCapabilitiesFrameHeader`、
     `RuntimeHealthFrameHeader`、`RuntimeRegisteredFrameHeader`、legacy
     `RuntimeRegisterFrameHeader`；通用 `encode_binary_frame` 可字节级构造
     outbound bootstrap/registered；
   - `protocol.rs`：closed `RuntimeFrameFamily` registry（6 family）、
     `FrameDirection`、`PayloadPresenceRule`、`RuntimeFrameSink`/sink bundle。
4. contracts-session corpus：`registration-handshake/frames.json` 12 帧
   byte-exact + 19 个场景（accept 与全部负例）；`process-lifecycle/
   shutdown-sequence.json`。
5. 目标 Register 序列/帧在 transport **未缺失**（§3.5 全部帧已存在且
   byte-exact），因此不阻塞等待 W-model；W-session 直接消费 canonical
   codec，不在 skiff-router 私建 codec 副本。
6. `RuntimeConnectionEpoch` / `RuntimeSessionEpoch` /
   `RuntimeRegistrationDirectory` / `RuntimeRegistrationTransition` 等
   production 类型全仓库不存在（仅 contract/test 参考模型），由本节点在
   `router/src/session/` 实现。
7. 当前 RouterConfig 没有 generation/assembly/configSnapshot 字段，Rust
   `ActiveRoutingEpochStore`/bootstrap 源尚未落地：`run_router` 装配的
   `SessionLayer` 无 committed epoch 时**不发出** `router.bootstrap`，连接
   停留在 Accepted 并在 bootstrap deadline 以 `BootstrapTimeout` fail-closed
   （不伪造 committed epoch）；真实 handshake probe 由测试注入 contracts
   corpus 的 committed epoch。bootstrap lane 合入后由集成 Agent 把 epoch
   source 接到 `SessionLayer`，不改变本节点文件边界。

## 任务目标

在 `router/src/session/` 实现 W-session：

- connection/session task：每 `/runtime` WS 连接一个 task，持有
  `RuntimeConnectionEpoch`，绑定后持有 `RuntimeSessionEpoch`；
- handshake 状态机：accept → `router.bootstrap` → `runtime.capabilities`
  → bind → `assembly.activation:Register` → transition 验证 → pending
  发布 → `runtime.registered` ACK → Registered → `runtime.health` 观察；
- `RuntimeRegistrationDirectory`：`current_by_replica` /
  `sessions_by_epoch` 双索引、replacement（先 cancel old 再 install new）、
  `RuntimeRegistrationTransition`（同 session post-commit re-register：
  exact duplicate 幂等 / new revision / new-generation 拒绝 / stale 关闭）、
  close barrier（全 ACK 后才删除 exact session，old finalizer 不删除
  replacement）；
- pre-auth 独立上限（默认 `runtime.maxConcurrency`）与
  bootstrap/capabilities/register 独立 deadline；
- per-session cancellation token + 静态 consumer manifest（reserved
  terminal slot）+ ACK barrier + fail-stop（barrier ACK 超时或 reserved slot
  失效 → 进程非零退出）；
- frame demux：消费 transport 的 closed family registry；Session family 的
  capabilities/health 与 `assembly.activation:Register`（RegistrationFrameSink
  adapter）可处理；legacy `runtime.register` 与未实现 family/transaction
  variant 终止 exact session（§6.1）；
- 真实 socket handshake probe：fake Runtime peer 按 corpus 逐字节发送，
  断言 wire 上 bootstrap/registered 与 fixture 字节一致、负例关闭连接且
  directory 归零；
- listener/main 的 session 装配：`/runtime` upgrade 后进入 SessionLayer，
  shutdown 时先停 accept 再经 S6 barrier 关闭 session，超时 fail-stop。

## 实现决策（在冻结契约语义内）

1. `SessionLayer` 是 W-session 的装配 owner（invariant：唯一拥有 directory、
   pre-auth pool、session task handles、fail-stop 标志；不拥有业务 routing/
   pending）。名字满足计划 §3.2 “Manager/Registry 等名称必须补充 invariant”。
2. `RuntimeRegistrationDirectory` 用 `std::sync::Mutex` 保护，操作不跨
   `.await`；replacement 由 `publish_pending` 原子标记 old cancelled 并返回
   old epoch，由新 session task 经 layer 触发 old session 的 close protocol。
3. close protocol 统一适用于 pending（RegisterValidated）与 Registered
   record：cancel token → reserved terminal slot 投递 `RuntimeSessionClosed`
   → consumer 幂等清理并 ACK → barrier 全 ACK 后删除 exact session；
   pre-ACK terminal 即回滚 pending（corpus `registeredSessions`/`revision`
   归零）。consumer mailbox 使用独立 bounded terminal channel（容量 1，
   `try_send`），data 满不影响 terminal 投递；投递失败或 ACK 超时 → fail-stop。
4. `RuntimeRegistrationTransition` 捕获 committed epoch；Registered 阶段
   收到与现有 tuple 不同的新 tuple 时按 §3.2 执行 transition（tuple ==
   current → publish new revision；tuple == pending → 拒绝；否则 stale
   关闭）。corpus 参考状态机未覆盖 post-commit 新 generation 场景，以权威
   设计 §3.2/§3.3 与 C-session §3.3 为准（该链 contract 明确要求 transition
   sequence test）。
5. 队列与预算：per-session outbound queue（frame 256 / 4 MiB，默认）由
   独立 writer task 消费（`WebSocketStream::split`），owner 只 non-blocking
   `try_send`；queue full/写失败 → `BootstrapWriteFail`/`AckLoss` strict
   terminal 并经独立 abort handle 关闭 socket；inbound frame/byte 预算
   （64 / 1 MiB，默认）超限 abort exact session。默认值为进程级常量，
   测试可注入更小值。writer 有独立 close 信号：关闭时放弃 pending 帧、立即
   写 close 应答（客户端 close handshake 可完成），500ms 未 drain 则 abort。
6. 除 frozen terminal 分类外，实现级 terminal 增加（文档化，不改变 corpus
   语义）：`MalformedFrame`（非 binary / 无法 decode / 方向违规）、
   `UnimplementedFamily`（已知 family 无 installed sink，§6.1）、
   `IngressBudgetExceeded`、`OutboundBudgetExceeded`、`RegistrationRefused`
   （consumer permit 获取失败，session 永不发布）。无 committed epoch 不
   新增 terminal：bootstrap 未写出 → bootstrap deadline → `BootstrapTimeout`。
7. 健康观察：`RuntimeHealthLedger` 只保存 observation 计数与按 session 的
   observation 清理（不持有 permit/socket/eligibility）；ACK 前 health
   丢弃并计数 `health_before_ack`，绝不进入 registered observation。
8. consumer terminal mailbox 的保留容量 = `runtime.maxConcurrency`
   （`PreAuthPool` 上限），保证“同时关闭最大允许 session 数”时全部 terminal
   可 non-blocking 入队、pending 归零；stuck consumer 仍会耗尽容量触发
   reserved-slot fail-stop（测试用容量 1 + parked consumer 验证）。
9. listener 装配：`start_listeners` 始终创建 `Arc<SessionLayer>`（配置
   驱动）；`ListenerStartOptions` 保持 PR 0b 三字段不变，测试经
   `start_listeners_with_session` 注入 corpus epoch/fake manifest/timing。
   `RouterListeners::shutdown` 顺序：先停 accept（S1）→ `SessionLayer`
   经 barrier 关闭全部 session（S6，总 deadline 20s）→ join listener
   tasks → fail-stop 时返回错误（main 非零退出）。
10. `router/Cargo.toml` 增加既有 workspace 依赖（无新版本、不扩 lock）：
    production `skiff-artifact-model`（消费 canonical `AssemblyActivationControl`/
    `RuntimeAssemblyRef`/`RuntimeConfigSnapshotRef`，M0 closure 不含宽
    runtime-model）+ dev `serde`（测试 fixture derive）。

## 写入边界

可写：

- `router/src/session/`（仅本节点）；
- `router/src/lib.rs`（仅 `mod session` + 必要 re-export）；
- `router/src/listener.rs`、`router/src/main.rs`（仅 session 装配）；
- `router/Cargo.toml`（仅上述既有 workspace 依赖声明）；
- `router/tests/`（新增 `session_*` 前缀测试文件；如 API 装配需要，允许
  最小更新既有 listener 测试的构造点）；
- `doc/implementation/router-rust-migration/execution/router-rust-migration-w-session-leaf.md`。

禁止：

- `router/src/artifact/`、`router/src/activation/`、`deployment`、
  `runtime/transport/src`、verify 注册表/selector graph/verify.yml、
  AGENTS.md、scripts README、`scripts/skiff-instance.mjs`；
- 在 skiff-router 私建 transport codec 副本；
- 操作 stable instance / Mongo / PM2 / 4004-4007；
- 跑全量 `pnpm verify`。

## 自验收矩阵

| 项 | 命令 / 证据 |
| --- | --- |
| session 测试（状态机/directory/barrier/demux/probe） | `cargo test --package skiff-router session`（含 `session_*` 文件） |
| 真实 socket handshake probe | 同上；fake Runtime peer 按 corpus 逐字节，bootstrap/registered 与 fixture 字节一致 |
| 聚焦 verify | `node scripts/verify.mjs --only router-rust,router-rust-process-smoke` |
| 既有 router 测试不回归 | `cargo test --package skiff-router` |
| 格式/clippy | `cargo fmt --check`、`cargo clippy --package skiff-router --all-targets`（exit 0） |
| 写集干净 | `git status` 仅本叶子声明文件；`git diff main...HEAD` 聚焦 |

## 交接

完成后向 `/root/router_rust_integration_b4` 报告 branch、worktree、
implementation commit/tree、实际写集、自验收矩阵与已知 seam
（`BootstrapUnavailable` 等待 bootstrap lane 接入 epoch source），并通知
root（父 Agent）。
