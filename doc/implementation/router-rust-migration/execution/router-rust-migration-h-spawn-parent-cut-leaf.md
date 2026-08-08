# Router Rust Migration Batch 6 — H-spawn-parent-cut Leaf Task

日期：2026-08-02
状态：execution leaf（一次性有界开发会话）
Agent：`/root/dev_h_spawn_parent_cut`
集成目标：`/root/router_rust_integration_b6`

## 引用链

- 直接父批次：`doc/implementation/router-rust-migration/execution/router-rust-migration-batch-6.md`
  （H-spawn-parent-cut 节点；baseline `main@8cabf352`；当前在
  `integration/router-rust-migration-batch-6` 分支，本叶子按路径引用）。
- 权威设计：`doc/implementation/router-rust-migration/execution/router-rust-migration-plan.md`（draft v5），
  重点 §5.3（C-model-spawn → W-model-spawn → M-spawn →
  H-spawn-parent-cut；`callerKind = request | actorInvocation` 决策；删除
  旧 shape 且无兼容 reader；C-spawn 在 hard-cut 后才解锁）、§5.4
  （`FunctionSpawnParentResolver` + `ActorSpawnParentResolver` + stateless
  `SpawnSubmitRouter` 按 exact parent kind 选择；collision / parent
  terminal / replacement 竞态 fail closed；sink 不拥有 pending）、§5.5
  （`SpawnSubmitRouter` 属于 stable sink bundle）。
- 冻结契约：
  - `doc/implementation/router-rust-migration/contracts/router-rust-migration-c-model-spawn-contract.md`
    （目标 wire：closed `callerKind`、`SpawnSubmitRequestFrameHeaderV2`、
    legacy-cut 规则；corpus：`runtime/transport/testdata/spawn-wire/`，5 帧
    + 10 场景；错误码 closed set 归 W-actor 消费）。
  - `doc/implementation/router-rust-migration/contracts/router-rust-migration-c-spawn-contract.md`
    （resolver / `SpawnSubmitRouter` 模型边界与 cut 前置；本节点只切 wire
    与 exact parent-kind 选择，不实现 W-actor handler）。
- W-model 交付物：`runtime/transport/src/protocol/spawn.rs`（canonical
  codec：`SpawnCallerKind` / `SpawnTargetKind` /
  `SpawnSubmitRequestFrameHeaderV2` / encode/decode 帧级函数；legacy 帧
  decode 拒绝）+ `router/tests/w_model_spawn_consumer.rs` /
  `runtime/tests/w_model_spawn_consumer.rs` consumer gate 先例。
- 先例：`doc/implementation/router-rust-migration/execution/router-rust-migration-h-registration-cut-leaf.md`
  （batch 5 H-cut 叶子：写集形态、自验收矩阵、driver 层硬切模式）。

## 零 worktree 只读预检结论（锚定 main@8cabf352）

1. baseline 锚定：`git rev-parse main` = `8cabf35289e87…`；worktree HEAD
   相同（`wt-h-spawn-parent-cut`，分支
   `feat/router-rust-h-spawn-parent-cut`）。
2. TS Router spawn 路径：
   - `runtimeEndpoint.ts` `spawn.submit.request` case 交给
     `RuntimeDispatcher.handleSpawnSubmit`；inbound 帧先过
     `validateRuntimeToRouterFrameHeader`（`runtimeProtocol.ts`），当前
     schema/validator 不要求 `callerKind`（`additionalProperties:false`
     会拒绝新字段）。
   - `runtimeDispatcher.ts::requireSpawnParent` 同时查 request pending 与
     actor invocation parent，两路都存在抛 ambiguous 错误——这正是设计要
     删除的“跨 namespace 猜测/歧义”形态。
   - `envelope.ts::SpawnSubmitRequestFrameHeader` 无 `callerKind`；
     fixture（`runtimeFrameHeaderFixtures`）无 `callerKind`。
   - Router 不构造 `spawn.submit.request`（inbound only）；outbound 是
     `spawn.submit.response/error`。
3. Rust Runtime spawn 路径：
   - eval `spawn_ops::submit_spawn_statement` 构造
     `SpawnSubmitControlRequest`（`runtime/request-contract`，无
     `callerKind`）；host `RuntimeOwnedRequestParts`（
     `runtime/host/src/eval_capability_adapter/actor.rs`）归一
     `caller_request_id`。
   - 普通 request 与 actor method execution 都经
     `actor_from_request`（`factory.rs`）构造 capability context；
     actor method 路径的 `RequestEnvelope.request_id` 是 invocation id
     （`actor_method_adapter.rs::ActorMethodEvalExecution::new`）。
   - 出站编码在 `runtime/host/src/host/router_session.rs::encode_writer_message`
     → `control_mapper::encode_outbound_control_message`（旧 shape
     `SpawnSubmitRequestFrameHeader`，无 `callerKind`）。
4. W-model codec consumer 用法：
   - `router/tests/w_model_spawn_consumer.rs` /
     `runtime/tests/w_model_spawn_consumer.rs` 已 byte-exact 消费 5 帧 +
     冻结 10 场景名；canonical decode 拒绝 legacy 帧。
   - `runtime/transport/tests/spawn_wire_corpus.rs` 含 resolver/router 参考
     模型（test-only），本叶子在 runtime consumer 侧复用同一语义。
5. 写边界发现（关键约束）：
   - `SpawnSubmitControlRequest` 构造点包含
     `runtime/transport/src/control_mapper/tests.rs`（禁写）。因此不能给
     该 struct 加字段（任何新字段都会让禁写文件编译失败）。
   - 方案：`callerKind` 走 `runtime/capability-context` 的
     `RouterWriterMessage` 新变体（host 自有 channel 消息，不触 transport）；
     driver 在 `encode_writer_message` 拦截并直接用 canonical V2 codec
     编码；`Control(OutboundControlMessage::SpawnSubmit)`（旧 shape）在
     driver 层 fail closed，生产零旧 shape outbound。
   - 契约错误码 closed set（`ParentNotFound` 等）归 W-actor 实现；本叶子
     保持 `spawn.submit.error` fail-closed（`SpawnSubmitRejected`），legacy
     帧在 endpoint 校验层拒绝（连接协议 terminal，与 H-registration-cut
     的 legacy 帧处理一致）。

## 任务目标（H-spawn-parent-cut，plan §5.3/§5.4/§5.5）

current TS Router 与 Rust Runtime 同时硬切 spawn 新 wire：

- `spawn.submit.request` 必须携带 required closed enum
  `callerKind = request | actorInvocation`（`SpawnSubmitRequestFrameHeaderV2`
  字段序与 corpus mirror 一致）。
- 删除旧 shape outbound：`targetKind + callerRequestId` 无 `callerKind`
  的 `spawn.submit.request` 不再由任何 production 构造/发送；无兼容 reader
  （不猜测、不默认 request、不跨 namespace 查找）。
- Router 按 `callerKind` 精确选择 parent namespace：`request` 只走 request
  pending（`FunctionSpawnParentResolver` 语义），`actorInvocation` 只走
  actor invocation pending（`ActorSpawnParentResolver` 语义）；同一字符串
  跨 namespace 不碰撞；parent terminal / replacement / connection /
  authority 不满足即 fail closed；accepted spawn 与 parent 生命周期分离。
- sink 不拥有 pending：`SpawnSubmitRouter` 语义由
  `RuntimeDispatcher.handleSpawnSubmit` 承担，不保存 parent-child 映射。

先让 TS/Rust consumer 过共享 corpus（新增 consumer 测试）再改 production；
不写兼容 reader/fallback。

## 实现决策（冻结契约语义内）

### TS Router（`router/src/`）

1. `protocol/envelope.ts`：`SpawnSubmitRequestFrameHeader` 增加 required
   `callerKind: SpawnCallerKind`（`'request' | 'actorInvocation'`）。
2. `protocol/runtimeProtocol.ts`：
   - `spawn.submit.request` schema：`callerKind` enum 属性 + required；
   - fixture 增加 `callerKind: 'request'`；
   - `validateSpawnSubmitRequest` 增加 `requireEnum(... 'callerKind',
     ['request', 'actorInvocation'])`（缺失/非法一律拒绝，legacy-cut）。
3. `router/runtimeDispatcher.ts`：`requireSpawnParent` 按
   `submit.callerKind` 精确选择：
   - `request` → 只查 request pending（
     `resolveSpawnRequestParent`，含 authority/connection/capability
     校验）；
   - `actorInvocation` → 只查 `activeActorInvocationParent`（
     `resolveSpawnActorParent`）；
   - 删除跨 namespace 查找与 ambiguous 分支；非法 kind 防御性拒绝。

### Rust Runtime（`runtime` crate driver/consumer）

4. `runtime/request-contract/src/outbound_control.rs`：新增 closed enum
   `SpawnCallerKind { Request, ActorInvocation }`（`as_str()`：
   `"request"` / `"actorInvocation"`），经 `outbound.rs` / `lib.rs` /
   `runtime/capability-context/src/outbound_control.rs` re-export。
5. `runtime/capability-context/src/outbound_control.rs`：新增
   `SpawnSubmitControlMessage { request: SpawnSubmitControlRequest, payload:
   Vec<u8>, caller_kind: SpawnCallerKind }` 与
   `RouterWriterMessage::SpawnSubmit(SpawnSubmitControlMessage)` 变体。
6. `runtime/host/src/eval_capability_adapter/actor.rs`：
   `RuntimeOwnedRequestParts` 增加 `spawn_caller_kind: SpawnCallerKind`；
   `submit_spawn`（borrow/owned 两个 impl）把
   `parts.spawn_caller_kind` 传给 `RequestClient::submit_spawn_in_scope`。
7. `runtime/host/src/eval_capability_adapter/factory.rs`：production
   `actor_from_request` 增加 `spawn_caller_kind` 参数；普通 request 装配
   传 `Request`，actor method 装配（`actor_method_adapter.rs`）传
   `ActorInvocation`，rebinder/assembly 上下文传 `Request`；
   `TestActorCapabilityFactory` 保持签名并内部传 `Request`。
8. `runtime/host/src/capability_context/actor.rs`：`ControlContext` 增加
   `send_spawn_submit`（两个 impl）；`RequestClient::submit_spawn(_in_scope)`
   增加 `caller_kind` 参数并发送 `RouterWriterMessage::SpawnSubmit`。
9. `runtime/host/src/host/router_session.rs`：
   - `encode_writer_message` 新 arm：`RouterWriterMessage::SpawnSubmit(msg)`
     → driver 侧 V2 映射（`SpawnSubmitControlRequest` + `caller_kind` →
     `SpawnSubmitRequestFrameHeaderV2`，字段序与 corpus 一致）→
     `encode_spawn_submit_request_frame`；
   - `Control(OutboundControlMessage::SpawnSubmit { .. })`（旧 shape）→
     fail-closed 错误（不产生任何旧 shape 帧）；
   - 映射含 service_id 校验（沿用 `publication_storage_segment`）、
     `caller_request_id` 必须 present、`target_kind` closed 映射、
     `actorMethod` 成对映射。

### 测试（consumer 先于 production 语义落地）

10. `router/tests/h_spawn_parent_cut_spawn_wire.test.ts`（新增）：共享
    corpus consumer——5 帧经 TS `encodeBinaryFrame`/`decodeBinaryFrame`
    byte-exact roundtrip（V2 header 字段序与 corpus 一致）；legacy 帧
    decode 无 `callerKind`（cut 标记）；10 场景经测试内参考模型 replay。
11. `runtime/tests/h_spawn_parent_cut_corpus.rs`（新增）：runtime crate
    consumer——5 帧 byte-exact + legacy 拒绝 + 10 场景参考模型 replay
    （collision / terminal / replacement / authority / target-kind）。
12. `router/tests/h_spawn_parent_cut_parent_kind.test.ts`（新增）：真实
    endpoint + dispatcher 生产场景 replay——request parent exact、
    actorInvocation parent exact、同一 id 双 namespace 不碰撞、legacy 帧
    连接 terminal、parent terminal / replacement / connection mismatch /
    authority mismatch 拒绝、accepted spawn 在 parent terminal 后继续、
    target-kind mismatch 拒绝。
13. `runtime/host/src/host/router_session/tests/h_spawn_parent_cut.rs`
    （新增）：driver 全链路——`SpawnSubmit` 消息编码为 V2（callerKind
    request / actorInvocation 分别断言）；legacy `Control(SpawnSubmit)`
    fail closed；full-loop duplex 收到 `spawn.submit.response/error`。
14. 既有测试更新：TS `helpers/actorRoutingHarness.ts::spawnSubmit` 增加
    callerKind；`protocol.test.ts` / `assembly-replica-dispatch.test.ts` /
    `runtime-dispatcher-self-ingress-actor-parent.test.ts` 等构造
    `spawn.submit.request` 处补 callerKind；Rust
    `eval_capability_adapter/actor/tests.rs`、
    `capability_context/actor/tests.rs`、
    `router_session/tests.rs`、`control_response_lifecycle.rs` 的消息捕获
    改新变体。

## 写集（全部在 worktree `/Users/geek/workspace/wt-h-spawn-parent-cut`）

TS production（`router/src/`）：

1. `router/src/protocol/envelope.ts`。
2. `router/src/protocol/runtimeProtocol.ts`。
3. `router/src/router/runtimeDispatcher.ts`。

TS tests（`router/tests/`）：

4. `router/tests/h_spawn_parent_cut_spawn_wire.test.ts`（新增）。
5. `router/tests/h_spawn_parent_cut_parent_kind.test.ts`（新增）。
6. `router/tests/helpers/actorRoutingHarness.ts` 及既有构造
   `spawn.submit.request` 的测试文件（`protocol.test.ts`、
   `assembly-replica-dispatch.test.ts`、`runtime-dispatcher-self-ingress-actor-parent.test.ts`
   等按编译/行为需要更新）。

Rust runtime（`runtime` crate src + tests）：

7. `runtime/request-contract/src/outbound_control.rs`、
   `runtime/request-contract/src/outbound.rs`、
   `runtime/request-contract/src/lib.rs`（`SpawnCallerKind` re-export）。
8. `runtime/capability-context/src/outbound_control.rs`、
   `runtime/capability-context/src/lib.rs`（`SpawnSubmitControlMessage` /
   `RouterWriterMessage::SpawnSubmit` / re-export）。
9. `runtime/host/src/eval_capability_adapter/actor.rs`、
   `runtime/host/src/eval_capability_adapter/factory.rs`、
   `runtime/host/src/eval_capability_adapter/actor_method_adapter.rs`、
   `runtime/host/src/eval_capability_adapter/assembly_execution_context.rs`、
   `runtime/host/src/eval_capability_adapter/activation_execution_rebinder.rs`。
10. `runtime/host/src/capability_context/actor.rs`。
11. `runtime/host/src/host/router_session.rs`（driver encode 拦截）。
12. `runtime/host/src/host/router_session/tests/h_spawn_parent_cut.rs`（新增）。
13. `runtime/host/src/host/router_session/tests.rs`、
    `runtime/host/src/host/router_session/tests/control_response_lifecycle.rs`、
    `runtime/host/src/eval_capability_adapter/actor/tests.rs`、
    `runtime/host/src/capability_context/actor/tests.rs`（消息捕获更新）。
14. `runtime/tests/h_spawn_parent_cut_corpus.rs`（新增）。

doc：

15. `doc/implementation/router-rust-migration/execution/router-rust-migration-h-spawn-parent-cut-leaf.md`
    （本文件）。

禁止写：`runtime/transport/src`、`router/src/session/`（Rust）、deployment、
AGENTS.md、scripts README、verify 注册表/selector graph/verify.yml、
`scripts/skiff-instance.mjs`、`Cargo.toml`/`Cargo.lock`（本节点不需要新
依赖）。

## 自验收矩阵

| 项 | 证据 |
| --- | --- |
| router TS tests 全绿 | `pnpm --filter @skiff/router test`（worktree 内 vitest run） |
| runtime tests 全绿 | `cargo test -p runtime`（含 host driver 测试与 `runtime/tests/h_spawn_parent_cut_corpus.rs`） |
| 共享 corpus consumer 测试 | TS `h_spawn_parent_cut_spawn_wire.test.ts` + Rust `h_spawn_parent_cut_corpus.rs` 全绿；5 帧 byte-exact；10 场景全 replay |
| 生产场景 fail closed | TS `h_spawn_parent_cut_parent_kind.test.ts` + Rust driver 测试全绿（collision / terminal / replacement / authority / legacy） |
| 旧 shape production 零命中 | `rg` 反向搜索：`SpawnSubmitRequestFrameHeader`（旧 DTO）在 `runtime/host`、`runtime/eval`、`router/src` production 零命中；`OutboundControlMessage::SpawnSubmit` 的 `Control(...)` 出站路径 production 零命中（driver 拦截并拒绝）；TS 无无 `callerKind` 的 `spawn.submit.request` 构造/校验 |
| transport 未触碰 | `git diff main...HEAD -- runtime/transport` 为空 |
| 写集干净 | `git status` 仅本叶子写集；`git diff main...HEAD` 聚焦 |

不跑全量 `pnpm verify`；不操作 stable instance/Mongo/PM2/4004-4007。
`CARGO_TARGET_DIR=/Users/geek/workspace/wt-h-spawn-parent-cut/target`。

## 交接

完成后向 `/root/router_rust_integration_b6` 报告 branch、worktree、
implementation commit/tree、实际写集、自验收矩阵，并通知 root（父 Agent）。
