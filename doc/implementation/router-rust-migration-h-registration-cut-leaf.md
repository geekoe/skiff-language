# Router Rust Migration Batch 5 — H-registration-cut Leaf Task

日期：2026-08-02
状态：execution leaf（一次性有界开发会话）
Agent：`/root/dev_h_registration_cut`
集成目标：`/root/router_rust_integration_b5`

## 引用链

- 直接父批次：`doc/implementation/router-rust-migration-batch-5.md`
  （H-registration-cut 节点；baseline `main@85596193`）。
- 权威设计：`doc/implementation/router-rust-migration-plan.md`（draft v5），
  重点 §2.5、§3.5（真实 Runtime handshake 合同）、§3.6、
  §5.3（M-registration 后硬切）。冲突时以权威设计为准。
- 冻结契约：
  - `doc/implementation/router-rust-migration-c-model-registration-contract.md`
    （corpus：`runtime/transport/testdata/registration-handshake/`，12 帧 +
    19 场景；strict terminal 分类 §2.3；health 观察规则 §2.4）。
  - `doc/implementation/router-rust-migration-c-session-contract.md`
    （pre-auth 上限、握手 timeout、pending/ACK 语义）。
- 参考实现（禁写、只读语义参考）：`router/src/session/handshake.rs`、
  `router/src/session/task.rs`、`router/src/session/demux.rs`（W-session 已按
  同一契约实现的 Router 侧握手驱动）。
- W-model 交付物：`runtime/transport/src/protocol/session.rs` 等 frame 级
  codec + `router/tests/w_model_registration_consumer.rs` /
  `runtime/tests/w_model_registration_consumer.rs` consumer gate 先例。

## 零 worktree 只读预检结论（锚定 main@85596193）

1. baseline 锚定：`git rev-parse main` = `85596193…`，worktree HEAD 相同。
2. TS Router 现状：
   - `RuntimeEndpoint`（`router/src/router/runtimeEndpoint.ts`）在
     `runtime.register` case 中通过 `RuntimeRegistry.registerRuntime` 接受
     inbound legacy 注册并回 `runtime.registered`；assembly 模式下
     `assembly.activation:Register` 由 `AssemblyRuntimeRegistry.register`
     处理，但**不发送 registered ACK**（与目标 §3.5 第 7 步不一致）。
   - `runtime.capabilities` 已作为独立帧存在；health 在 legacy 模式下经
     `recordRuntimeHealth`、assembly 模式下经 `AssemblyRuntimeRegistry.recordHealth`
     记录，无 ACK 前丢弃规则。
   - `server.ts` 生产装配为 `AssemblyRuntimeRegistry` +
     `RuntimeEndpoint(assemblyRegistry)` 模式；legacy 服务级注册只被
     非 assembly 测试路径使用。
3. Rust Runtime 现状（`runtime/host/src/host/router_session.rs`）：
   - bootstrap 到达后 `queue_connection_registration` 发送
     `runtime.capabilities` + `assembly.activation:Register`；
   - `runtime.registered` 只要求 bootstrap 先到，**不校验 ACK runtime_id
     与自身 replica 一致**；business 帧只要求 bootstrap 先到（未要求
     Registered）；handshake 无显式 phase/terminal 分类。
   - health 已经只在 ACK 后由 `RuntimeHealthReporter.record_registered`
     开启（满足“health 不能在 ACK 前被当作 registered observation”）。
4. 契约/corpus 差异表（C-model-registration §4）与 W-model codec 均收敛：
   - TS `encodeBinaryFrame`/`decodeBinaryFrame` 与
     `encodeAssemblyActivationFrame('runtimeToRouter', …)` 对 corpus 全部
     12 帧 byte-exact roundtrip（本叶子实测通过）。
   - `runtime.register` 不属于目标 handshake 帧；`H-registration-cut` 删除
     inbound legacy `runtime.register`，`runtime.registered` 只作成功
     Register ACK。
5. 测试面：legacy wire 注册被 `helpers/runtime.ts`（`openRegisteredRuntime`
   /`MockRuntime.register`）、`helpers/routerHarness.ts`、`helpers/
   actorRoutingHarness.ts` 与若干非 assembly 测试直接使用。本叶子把
   helper 改为 capabilities（wire）+ `RuntimeRegistry.registerRuntime`
   （内存直调，无 wire legacy 字面量）；显式发送 legacy wire 帧的测试改为
   断言 `LegacyRegisterRejected`（连接关闭、零注册残留）。

## 任务目标（H-registration-cut，plan §3.5/§5.3）

current TS Router（`router/src` TS 生产 + `router/tests`）与 Rust Runtime
（`runtime` crate connection/handshake driver）同时硬切到新 handshake：

```text
accept RuntimeConnectionEpoch
-> Router sends router.bootstrap
-> Runtime sends runtime.capabilities
-> bind RuntimeSessionEpoch / acquire installed-consumer permits
-> Runtime sends assembly.activation:Register
-> RuntimeRegistrationTransition 验证 committed epoch 并 publish routable revision
-> Router sends runtime.registered（registered ACK）
-> Runtime starts runtime.health
```

删除 inbound legacy `runtime.register`；`runtime.registered` 只作为成功
Register ACK；wrong order / identity change / duplicate / stale / ACK loss
严格 terminal；health 不能在 ACK 前被当作 registered observation。

先让 TS/Rust consumer 过共享 corpus（新增 consumer 测试）再改 production；
禁止写兼容 reader/fallback。

## 实现决策（冻结契约语义内，TS 侧对齐 W-session 参考实现）

1. `router/src/router/runtimeHandshake.ts`：TS 版 per-connection 纯状态机
   （Accepted → BootstrapSent → CapabilitiesBound → RegisterValidated →
   Registered → Closed），terminal 分类与 corpus §2.3 完全一致；事件
   `onBootstrapWritten/onBootstrapWriteFailed/onCapabilities/onRegister/
   onLegacyRegister/onHealth/onAckWritten/onAckWriteFailed/onTimeout/
   onDisconnect`。不持有 socket/directory/time，由 endpoint 驱动。
2. `RuntimeEndpoint`：
   - 连接时写 bootstrap（成功 → BootstrapSent；写失败 → BootstrapWriteFail
     terminal）；pre-auth 上限（新增 `preAuthMaxConcurrency`，生产传
     `runtime.maxConcurrency`）；bootstrap/capabilities/register/ack_write
     deadline（10s/10s/30s/5s，可注入）。
   - inbound：capabilities → bind；`assembly.activation:Register` →
     机器验证（exact current → RegisterValidated + pending publish → 写
     ACK；ACK 成功 → Registered + commit publish；ACK 失败 → AckLoss +
     rollback；pending tuple → NewGenerationBeforeEpochSwap；其他 →
     StaleRegister）；post-commit 同 tuple → Idempotent；新 committed tuple
     → Transition（再次 ACK）；legacy `runtime.register` →
     LegacyRegisterRejected；health → Observed（仅 Registered）/
     DroppedBeforeAck（RegisterValidated 计数丢弃）/ terminal（更早阶段）；
     business 帧在 Registered 前 → WrongOrder terminal。
   - `runtime.registered` 只由成功 Register 路径发出。
3. `AssemblyRuntimeRegistry`：`register` 拆为 `publishPending`（pending
   发布，不入 routable/health）/ `commitPending`（ACK 后转正）/
   `rollbackPending`（terminal 回滚）；保留 `register` 为直接调用的
   publish+commit 便捷入口（测试用，非 wire fallback）。
4. `RuntimeRegistry`：移除 `RuntimeRegisterEnvelope`/`RuntimeRegisterFrameHeader`
   依赖与 `runtime.register` 字面量；`registerRuntime` 保留为内部注册 API
   （结构性入参，无 wire type 字段），供 legacy 组件/测试直调；
   capabilities/health/fence 机制不变。
5. `router/src/protocol/envelope.ts`、`runtimeProtocol.ts`：删除 legacy
   `runtime.register` 帧类型、schema、validator、fixture（production 零命中）。
6. Rust Runtime（`runtime/host/src/host/router_session.rs`）：
   - 新增 client 侧握手状态机（WaitingBootstrap → BootstrapReceived →
     RegisterSent → Registered → Closed）与 terminal 分类；
   - bootstrap 只在 WaitingBootstrap 接受（重复 → WrongOrder terminal）；
     成功后 queue capabilities+register，writer 排空两帧后进入
     RegisterSent；`runtime.registered` 只在 RegisterSent 接受且
     runtime_id 必须等于自身 replica（否则 WrongOrder/IdentityChange
     terminal）；business/activation/WS-generation 帧只在 Registered 接受；
     inbound `runtime.capabilities`/`runtime.health`/`runtime.register`
     为方向/协议违例 terminal；disconnect/写失败 terminal；
   - health 仍只在 ACK 后开始（`RuntimeHealthReporter`），ACK 前 health
     帧不作为 observation；
   - `run_connected_session_with_bootstrap` 保持为测试快捷入口
     （等价已 Registered 状态），生产路径走严格状态机。

## 写集（全部在 worktree `/Users/geek/workspace/wt-h-registration-cut`）

TS production（`router/src/`）：

1. `router/src/router/runtimeHandshake.ts`（新增，纯状态机）。
2. `router/src/router/runtimeEndpoint.ts`（握手驱动接线、删 legacy case、
   ACK、pre-auth、deadline）。
3. `router/src/router/assemblyRuntimeRegistry.ts`（pending publish/commit/
   rollback；health 与 routable 只认 registered）。
4. `router/src/router/runtimeRegistry.ts`（去 `runtime.register` 字面量；
   `registerRuntime` 结构性入参）。
5. `router/src/protocol/envelope.ts`、`router/src/protocol/runtimeProtocol.ts`
   （删 legacy 帧类型/schema/validator/fixture）。
6. `router/src/router/server.ts`（`preAuthMaxConcurrency` 装配）。

TS tests（`router/tests/`，`h_registration_cut_*` 或既有文件内更新）：

7. `router/tests/h_registration_cut_handshake.test.ts`（新增：corpus 12 帧
   TS codec byte-exact roundtrip；19 场景 driver replay（terminal/计数/
   registered 断言）；真实 endpoint 集成：accept 序列 ACK byte-exact、
   legacy 拒绝、错序/identity/stale 关闭、ACK 前 health 不观察、pre-auth
   拒绝、re-register idempotent/stale）。
8. 既有文件更新：`helpers/runtime.ts`、`helpers/routerHarness.ts`、
   `helpers/actorRoutingHarness.ts`（注册 helper 改 direct registerRuntime +
   capabilities）；`protocol.test.ts`、`router-bootstrap-session.test.ts`、
   `assembly-runtime-endpoint.test.ts`、`runtime-registry-dispatch.test.ts`
   （显式 legacy wire 用例改拒绝断言）、`runtime-capability-session-fence.test.ts`、
   `actor-production-routing.test.ts` 等依赖 legacy 字面量的测试。

Rust Runtime（`runtime` crate src + tests）：

9. `runtime/host/src/host/router_session.rs`（严格门禁接线）+
   `runtime/host/src/host/router_session/handshake.rs`（client 侧 handshake
   状态机 + terminal 分类 + deadlines）。
10. `runtime/host/src/host/router_session/tests/h_registration_cut.rs`
    （新增；full-loop duplex + corpus 字节的握手/负例/健康门禁测试）。
11. `runtime/tests/h_registration_cut_corpus.rs`（新增；runtime crate
    consumer 直接消费同一 corpus：帧 roundtrip + 方向/场景一致性）。

doc：

12. `doc/implementation/router-rust-migration-h-registration-cut-leaf.md`
    （本文件）。

禁止写：`runtime/transport/src`、deployment、`router/src/session/`（Rust）、
AGENTS.md、scripts README、verify 文件、`scripts/skiff-instance.mjs`、
`Cargo.toml`/`Cargo.lock`（本节点不需要新依赖）。

## 自验收矩阵

| 项 | 证据 |
| --- | --- |
| router TS tests 全绿 | `pnpm --filter @skiff/router test`（或 worktree 内 vitest run） |
| runtime tests 全绿 | `cargo test -p runtime`（含 host full-loop 与 runtime/tests corpus consumer） |
| 共享 corpus consumer 测试 | TS `h_registration_cut_handshake.test.ts` + Rust `h_registration_cut_*` 全绿；corpus 帧 byte-exact |
| TS/Rust handshake wire 与 corpus 一致 | TS codec roundtrip 12 帧 + Rust transport corpus 既有测试不回归 |
| rg 负例 | legacy `runtime.register` **acceptance** 面零命中：envelope 类型、
  schema、validator、fixture、inbound register case、legacy 注册 API 全部删除；
  仅 strict-rejection 分类器保留字面量（`runtimeEndpoint.ts` 与
  `router_session/handshake.rs`/`router_session.rs` 的方向违例分类），与
  W-session `demux.rs` 的 `LegacyRegisterRejected` 分类一致（契约 §2.3 必需） |
| 写集干净 | `git status` 仅本叶子写集；`git diff main...HEAD` 聚焦 |

不跑全量 `pnpm verify`；不操作 stable instance/Mongo/PM2/4004-4007。
`CARGO_TARGET_DIR=/Users/geek/workspace/wt-h-registration-cut/target`。

## 交接

完成后向 `/root/router_rust_integration_b5` 报告 branch、worktree、
implementation commit/tree、实际写集、自验收矩阵，并通知 root（父 Agent）。
