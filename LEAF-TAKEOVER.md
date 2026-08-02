# LEAF-TAKEOVER: fix/spawned-actor-test-capability（接管 2026-08-02）

父节点：`.task-contracts/TAKEOVER-20260802.md` §3.2。仓库：`/Users/geek/workspace/skiff`，
worktree：`/Users/geek/workspace/skiff-spawned-actor-test-capability`，分支
`fix/spawned-actor-test-capability`，checkpoint commit `9bb6e8c4`。

## 预检事实（只读，2026-08-02 09:4x CST）

- 工作树仅剩 untracked `node_modules` / `router/node_modules` 符号链接，无其他脏改动。
- 磁盘可用 30 GiB（`df`），高于 10 GiB 阈值。
- Router 全量：64 files / 839 tests 全绿（`npx vitest run`）。
- Rust 聚焦：`skiff-runtime-eval actor_executor` 51 项全绿；`skiff-runtime-host actor_owner`
  12 项全绿；`skiff-runtime-transport actor_*` 26 项全绿。
- 真实 E2E fixture（`scripts/run-actor-full-chain-acceptance.mjs`）尚未在本接管会话复跑，
  计划在实现完成后跑一次聚焦验证。

## §3.2 七个缺口逐项预检结论

| # | 缺口 | 预检结论 | 证据 |
| --- | --- | --- | --- |
| 1 | 真实 Runtime WebSocket FIFO 回归 | **已实现** | `router/tests/runtime-endpoint-actor-message-fifo.test.ts`（真实 ws server + client，3 断言：terminal 前 child 被捕获、terminal 后拒绝、owner.invoke 先于 spawn.submit.response）；`handleSpawnSubmit` 同步前缀先 `requireSpawnParent`，terminal 在 `handleFrame` 同步前缀 `claimPendingInvocation` 移除，顺序可见；端点不串行 socket |
| 2 | generation 固定（owner.invoke/control wire authority + Host 精确 retained generation，fail closed） | **未实现** | `ActorOwnerInvokeFrameHeader`/`ActorOwnerControlFrameHeader`（TS+Rust）无 assemblyIdentity/generation；`runtime/host/src/host/actor_owner_execution.rs:525,653` 仍 `active_actor_execution_route(&service_id)` 取当前 active；`assembly_admission.rs` 只有单个 `active`，无 retained 历史；无 G1→G2 的 Host 侧测试 |
| 3 | request/Actor pending 同 ID 跨表碰撞 | **已实现** | `runtimeDispatcher.requireSpawnParent` 同时解析 request 与 actor 两个候选，双命中即拒绝；`activeTestCaseParent` XOR 只准一个；`runtime-dispatcher-self-ingress-actor-parent.test.ts` 两项碰撞测试通过 |
| 4 | RuntimeEndpoint admission 顺序同步可见（不串行整个 socket） | **已实现（实现层面）** | 端点 `ws.on('message')` 每帧独立异步处理、无整 socket 队列；admission 变更都在各 handler 同步前缀完成（`requireSpawnParent`、`claimPendingInvocation`、`reserveInvocation`）；FIFO 测试验证顺序。计划补一条“pending admission 不阻塞同 socket 其他帧”的聚焦测试以固化 |
| 5 | 无 create 分支 cancel/deadline 后不得 admit + 失败清理 + 同 fence 可重试 | **已实现** | `runtime/eval/src/actor_executor.rs` no-create 分支先过 ExecutionScope checkpoint 再 admit；`actor_executor/tests.rs` 8 个测试（pre-cancel/expired/drop/abort/panic/follower/session-close 等）全绿 |
| 6 | ActivateInitial pending-create 断连真实 WebSocket 清理测试 | **未实现** | `actor-runtime-disconnect.test.ts` 是 ActorManager 单元级；`actor-get-create-activation.test.ts` 用 fake socket；`assembly-runtime-endpoint.test.ts` 无 pending-create+断连用例。需补真实 WebSocket 用例 |
| 7 | HTTP schema 三态门禁 | **已实现** | `assemblyHttpGateway.readTestCaseCorrelationHeaders`：双头缺失→production、合法 pair→test、单边/重复/非法→400；`runtimeAssemblyRequest.validateTestCapability` 三态；`runtime-assembly-unary-dispatch.test.ts` 4 项 HTTP 门禁用例 + `protocol.test.ts` schema 用例全绿 |

## 实际写集（本接管会话）

- `LEAF-TAKEOVER.md`（本文件）。
- 缺口 2（generation 固定，主要实现）：
  - `runtime/transport/src/actor_owner.rs`：owner.invoke/control 增加
    `routeAuthority { assemblyIdentity, assemblyGeneration }` 必填 + 校验；
    更新 `actor_owner/tests.rs`。
  - `router/src/protocol/actorOwnerProtocol.ts`：同构必填字段 + 校验。
  - `router/src/router/actorMethodDispatcher.ts` + `productionActorMethodRouter.ts`：
    dispatch context 携带 routeAuthority，owner.invoke/control 发送时写入；
    新增 authority seam `actorOwnerRouteAuthority`（生产接 AssemblyRuntimeRegistry，
    测试 harness 提供）；普通路径解析不到即 fail closed。
  - `router/src/router/actorGetCreateActivationCoordinator.ts`：
    activateInitial control 写入 routeAuthority（capability 用 pinned lineage，
    ordinary 用 header.activationIdentity）。
  - `router/src/router/assemblyRuntimeRegistry.ts`：`actorOwnerRouteAuthority`。
  - `runtime/host/src/host/actor_route_holds.rs`（新增）：按存活 Actor owner
    执行（invoke/control）持有完整 ActiveAssembly 的 route-hold 注册表；
    Host 解析精确 (identity, generation)，active 或 held 之外一律 fail closed。
    该设计同时保留 WebSocket generation 的“pin 释放后旧 context 可回收”语义。
  - `runtime/host/src/loader/assembly_admission.rs`：`actor_execution_route`
    精确 active 查找 + 共享 `actor_route_from_active`。
  - `runtime/host/src/host/actor_owner_execution.rs`：invoke 任务与 owner control
    在解析 route 时 acquire hold，任务/控制结束释放；找不到 fail closed。
  - Host 新增 G1 挂起→G2 reload→G1 child 仍执行 G1 + missing/mismatched
    fail-closed 测试（真实 WebSocket session + create gate）。
- 缺口 6：真实 WebSocket pending-create（activateInitial 未 ack）断连清理测试
  （entry 保留为 inactive、lease 释放、同 fence 重试成功）。
- 缺口 4：补一条同 socket pending admission（两个并发 create + health）
  不阻塞其他帧的聚焦测试。
- 机械闭合：更新受影响的 TS/Rust 构造点与断言；
  `test-runner/src/bin/package_service_smoke_fixture.rs` 读取
  `SKIFF_TEST_INGRESS_URL` 并允许多 test case（分支 fixture 新增的两个 effects
  用例需要 ingressUrl；E2E 才能 PASS）。

## 自验收矩阵（聚焦）

| 项 | 命令/证据 |
| --- | --- |
| Router TS 全量 | `npx vitest run`（router）：全绿（含新增 FIFO/generation/断连用例） |
| Rust transport actor_owner | `cargo test -p skiff-runtime-transport --lib actor_owner`：全绿 |
| Rust host 全量 | `cargo test -p skiff-runtime-host --lib`：384 项全绿（含新增 G1→G2 hold 测试、websocket generation 回归） |
| Rust eval actor_executor（缺口 5 回归） | `cargo test -p skiff-runtime-eval --lib actor_executor`：全绿 |
| 真实 E2E fixture | `node scripts/run-actor-full-chain-acceptance.mjs`：PASS（2 replicas；含 spawned actor effects） |
| 缺口 1 FIFO 真实 WS | `runtime-endpoint-actor-message-fifo.test.ts`：PASS |
| 缺口 2 generation 固定 | wire routeAuthority 断言（actor-test-capability-authority / assembly-runtime-endpoint）+ Host G1→G2 hold 测试 + fail-closed：PASS |
| 缺口 3 碰撞 | `runtime-dispatcher-self-ingress-actor-parent.test.ts` 碰撞用例：PASS |
| 缺口 4 顺序可见不串行 | 新增同 socket 并发 create + health 用例：PASS |
| 缺口 6 断连清理 | 新增真实 WS pending-create 断连用例：PASS |
| 缺口 7 HTTP 三态 | `runtime-assembly-unary-dispatch.test.ts` + `protocol.test.ts`：PASS |
| rustfmt | `cargo fmt --check -p skiff-runtime-host -p skiff-runtime-transport -p skiff-test-runner`：通过 |

## 停止条件

- 需要改变公共契约/架构语义、新增语言概念、触碰兄弟任务 owner，或发现与既有 actor
  机制冲突时，先上报，不自行扩张。本文件列出的 wire 字段新增属于缺口 2 明确要求的
  “wire 携带 immutable route authority”，不视为 scope 扩张。

## 交叉情报 blocker（/root 2026-08-02 转达，最终验收前置）

stable（main agine + 当前 dirty skiff main runtime，gen59）出现平台级 Host 激活回归：
Host WS 能 open，但 service 的 connect handler 不再消费 activation token；新签 token
的 `/host/hello` 立即 `invalid_activation_token`（token 保持 pending）。证据：
`agine-acceptance-results/p104-long-command-progress-20260802-r2/`、P1-04 分支
`LEAF-TAKEOVER.md` §blocker；判别诊断在恢复 main 后复现。stable runtime 二进制与
skiff main build（04:59，hash f4c3b8d5…）逐字节一致，与本 worktree 构建
（aee40018…）不同——本分支未部署到 stable，也不是本分支引入。

### 与本分支的覆盖关系

- 本分支（4d260147..075918e2 全量）未改动 `std/websocket.skiff`、`prelude/`、
  `runtime/host/src/host/websocket_generation.rs`、`router/src/router/webSocketRequestBrokerState.ts`
  等 WebSocket connect/activation-token 消费路径；七个缺口（generation authority、
  RuntimeEndpoint admission、WS FIFO、actor owner wire、pending-create 断连等）均围绕
  actor owner/spawn 与 assembly authority，不覆盖 host hello 的 token 消费回归。
- 本分支代码也**未**改动 agine 侧 connect/activation 交互；token 字符串
  `invalid_activation_token` 在 skiff 仓库无命中，属于 internals（agine）服务侧。
- 因此：本分支合入不会修复该回归；该回归也不阻断本分支收口，但按 /root 要求最终验收前
  必须解决。

### 最小修复 owner 判断（本 Agent 判断）

首选 owner：**internals（agine）service 侧** —— Host connect handler 的 activation
token 消费/置为 pending 的流程（P1-04 分支共享的 `host_tool_reconciliation.skiff`、
`host_tool_settlement*.skiff`、`model.skiff` 或 host activation store），由 §3.3/§3.4
合并链上的 owner（presence/settlement 共享 owner）或 P1-04 分支 owner 在集成后收敛。
次选排查：skiff main dirty 工作树的 Router WebSocket request broker / runtime
`websocket_generation` 相关未提交改动（late-terminal 竞态、activation+admission-rank、
timeout 链），若判别诊断显示 token pending 记录在 router 侧则以 skiff main 集成 Agent
为 owner。本分支不认领该 blocker。
