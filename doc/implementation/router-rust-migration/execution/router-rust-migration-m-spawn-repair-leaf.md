# Router Rust Migration — M-spawn-repair Leaf Task

日期：2026-08-03
状态：execution leaf（一次性有界开发会话）
Agent：`/root/dev_m_spawn_repair`
集成目标：`/root/router_rust_integration_b9`

## 引用链

- 权威设计：`doc/implementation/router-rust-migration/execution/router-rust-migration-plan.md`（draft v5），
  重点 §5.3（C-model-spawn → W-model-spawn → M-spawn →
  H-spawn-parent-cut；`callerKind` 决策）、§5.4（C-spawn + M-spawn +
  H-spawn-parent-cut；typed parent namespace；sink 不拥有 pending）、
  §6.1（接口变化先改 contract/corpus/sequence test，再更新 consumer；
  incomplete handler 收到未实现 family 必须终止 exact Runtime session）。
- 冻结契约：`doc/implementation/router-rust-migration/contracts/router-rust-migration-c-model-spawn-contract.md`
  （方向标注自相矛盾，本叶子按权威设计修复）、
  `doc/implementation/router-rust-migration/contracts/router-rust-migration-c-spawn-contract.md`
  （`SpawnSubmitRouter` / `SpawnSubmitAcceptance` 边界）。
- 同链叶子：`router-rust-migration-w-model-actor-spawn-leaf.md`（W-model
  codec/corpus 交付）、`router-rust-migration-h-spawn-parent-cut-leaf.md`
  （TS Router / Rust Runtime production hard cut）。
- 批次：`doc/implementation/router-rust-migration/execution/router-rust-migration-batch-9.md`
  （E-gates wiring；基线 origin/main@acd47cfc；共享主 worktree 只读）。

## 零 worktree 只读预检结论（锚定 origin/main@acd47cfc）

1. baseline 锚定：`git rev-parse origin/main` =
   `acd47cfc8509d66c526ae105782546cc4f382c22`；worktree HEAD 相同
   （`wt-m-spawn-repair`，分支 `feat/router-rust-m-spawn-repair`）。
2. 五方事实交叉验证（方向与帧集）：
   - TS Router（`router/src/router/runtimeEndpoint.ts`：`spawn.submit.request`
     case 走 `validateRuntimeToRouterFrameHeader` + `dispatcher.handleSpawnSubmit`，
     inbound；`runtimeDispatcher.ts::requireSpawnParent` 按 `callerKind` 精确
     选择 parent namespace，返回 `spawn.submit.response` / `spawn.submit.error`
     outbound 给同一 connection）。
   - Rust Runtime host（`runtime/host/src/host/router_session/spawn_submit.rs`
     + `router_session.rs::encode_writer_message`）：`RouterWriterMessage::SpawnSubmit`
     编码 canonical `spawn.submit.request`（Runtime → Router 出站），inbound
     只解码 `spawn.submit.response` / `spawn.submit.error`（按 `rpcId`
     correlation 分发到 control response/error）。
   - transport registry（`runtime/transport/src/protocol.rs`）：
     `RuntimeFrameFamily::Spawn.direction()` 错误标注 `RouterToRuntime`。
   - session demux（`router/src/session/demux.rs`）：因 family direction 为
     RouterToRuntime，任何 inbound spawn 帧都终止；Spawn 落入
     `Unimplemented` 分支。
   - C-model-spawn corpus（`runtime/transport/testdata/spawn-wire/frames.json`）：
     全部帧标注 `RouterToRuntime`；其中三个 `spawn.submit.request` 帧与
     production 事实矛盾。
3. 结论（修复后的 canonical 事实）：
   - `spawn.submit.request`：RuntimeToRouter（唯一 inbound 方向）；
   - `spawn.submit.response` / `spawn.submit.error`：RouterToRuntime；
   - spawn family 为 mixed-direction：family 级 `Either` + 帧级 direction
     表（复用 Session/Request/Actor 的 Either + 帧级收窄模式）；
   - 不存在额外的 Router→Runtime forwarding/accept 帧：accept/reject 就是
     `spawn.submit.response` / `spawn.submit.error`，correlation 为 `rpcId`
     （response 额外携带 `spawnId` + `requestId`）；
   - Runtime driver 出站 bytes 已与 corpus `frameHex` 逐字节一致
     （`runtime/host/.../tests/h_spawn_parent_cut.rs` 已有 corpus byte-exact
     断言），无需改生产编码逻辑。
4. 数据面缺口：C-spawn §3.3 的 `SpawnSubmitAcceptance { spawn_id,
   request_id, status }` 在代码中不存在，且契约定义不携带真实执行 sink
   重建出站 `spawn.submit.request` 所需的原始 wire header/payload
   （service/activation identity、actor_method 元数据、args bytes）。
   E-actor-rust 前置要求补 typed 投影。

## 任务目标（修复冻结契约自相矛盾 + M-spawn 数据面缺口）

按权威设计 §6.1 工作流合入规则：**先更新 canonical contract/corpus，再改
所有消费者**。

1. canonical contract + corpus：
   - `c-model-spawn-contract.md`：方向事实改为帧级
     （request=RuntimeToRouter；response/error=RouterToRuntime；family 级
     Either + 帧级 direction 表）；`SpawnSubmitAcceptance` 数据面补
     raw wire header/payload 投影；correlation 形态写明。
   - `spawn-wire/frames.json`：三个 `spawn.submit.request` 帧方向改
     `RuntimeToRouter`；response/error 保持 `RouterToRuntime`；`frameHex`
     与 payload 逐字节不变。
2. transport registry / codec：
   - `protocol.rs`：`RuntimeFrameFamily::Spawn.direction()` →
     `Either`（family 级），注释说明帧级收窄。
   - `protocol/spawn.rs`：新增帧级 direction 表
     `spawn_submit_frame_direction(frame_type)`；新增
     `SpawnSubmitRequestFrame { header, payload }`（decodeAs 目标）与
     `SpawnSubmitAcceptance { request, spawn_id, request_id }`
     （含 `response_header()` 投影）。
3. session/demux：接受来自 Runtime 的 `spawn.submit.request`（canonical
   decode → `DemuxEvent::SpawnSubmit`）；response/error inbound 为方向违例
   → MalformedFrame；legacy/非法 request → MalformedFrame；task 未注入 sink
   时严格终止（`UnimplementedFamily`），生产装配由 E-gates wiring 接入真实
   spawn execution sink。
4. 受影响消费者测试全部同步方向断言（router/runtime/transport 的
   w_model / h_spawn_parent_cut / spawn corpus 测试），golden bytes 除方向
   标注外不变。
5. runtime-host spawn_submit：预检确认与 corpus 一致（已有 driver byte-exact
   测试），本叶子不改生产编码；仅在叶子验收中复跑该测试并记录。

## 写集（全部在 worktree `/Users/geek/workspace/wt-m-spawn-repair`）

canonical contract / corpus：

1. `doc/implementation/router-rust-migration/contracts/router-rust-migration-c-model-spawn-contract.md`。
2. `runtime/transport/testdata/spawn-wire/frames.json`（仅 direction 标注；
   `frameHex`/payload/header 不动）。

transport production（仅 spawn 相关）：

3. `runtime/transport/src/protocol/spawn.rs`（direction 表 +
   `SpawnSubmitRequestFrame` + `SpawnSubmitAcceptance` +
   `response_header()`；unit tests）。
4. `runtime/transport/src/protocol.rs`（Spawn family direction → Either；
   re-export 新 surface）。

router session demux（仅 Spawn 处理）：

5. `router/src/session/demux.rs`（帧级方向 + decode + `DemuxEvent::SpawnSubmit`）。
6. `router/src/session/task.rs`（仅一个匹配臂：`DemuxEvent::SpawnSubmit(_)`
   → `TerminalKind::UnimplementedFamily`，E-gates wiring 替换为真实 sink）。

corpus / consumer 测试（方向断言同步 + 新验收）：

7. `runtime/transport/tests/spawn_wire_corpus.rs`。
8. `runtime/transport/tests/w_model_spawn_corpus.rs`。
9. `runtime/tests/w_model_spawn_consumer.rs`。
10. `runtime/tests/h_spawn_parent_cut_corpus.rs`。
11. `router/tests/w_model_spawn_consumer.rs`。
12. `router/tests/spawn_repair_direction.rs`（新增：demux 方向 + legacy 拒绝 +
    response/error 方向违例 + 无 sink 终止语义）。
13. `router/tests/spawn_repair_acceptance.rs`（新增：`SpawnSubmitAcceptance`
    数据面——raw wire header/payload byte-exact 重建 + response 投影）。

doc：

14. `doc/implementation/router-rust-migration/execution/router-rust-migration-m-spawn-repair-leaf.md`
    （本文件）。

禁止写：其他 family 的 transport 模块、`control_mapper.rs`、http/listener/
supervisor、router TS（`router/src` 除上述两个 session 文件、`router/tests`
除上述新增文件）、deployment、AGENTS.md、scripts README、verify 注册表/
selector graph/verify.yml、`scripts/skiff-instance.mjs`、`Cargo.toml` /
`Cargo.lock`。

## 自验收矩阵

| 项 | 证据 |
| --- | --- |
| canonical contract/corpus 先行 | commit 顺序：contract + frames.json 在 production 改动之前；`git diff` 审计方向字段 |
| golden bytes 不变 | 所有 corpus 测试仍断言 `encode(decode(hex)) == hex`；frames.json 仅 direction 值变化 |
| transport 全绿 | `cargo test -p skiff-runtime-transport` |
| router demux 全绿 | `cargo test -p skiff-router --test spawn_repair_direction --test spawn_repair_acceptance --test session_demux --test w_model_spawn_consumer` |
| runtime consumer 全绿 | `cargo test -p runtime --test w_model_spawn_consumer --test h_spawn_parent_cut_corpus` |
| runtime host driver 对齐 | `cargo test -p runtime host::router_session::tests::h_spawn_parent_cut`（corpus byte-exact 保持） |
| 其他 family 未触碰 | `git diff origin/main...HEAD -- runtime/transport/src` 仅 spawn.rs/protocol.rs 的 Spawn 行 |
| 写集干净 | `git status` 仅本叶子写集；无 target/本地状态提交 |

## TS 侧影响面（router TS 禁写，明确上报）

- `router/tests/h_spawn_parent_cut_spawn_wire.test.ts` 第 285 行断言所有
  corpus 帧 `direction === 'RouterToRuntime'`；corpus 修复后该断言按新契约
  应改为帧级期望（request=RuntimeToRouter）。本叶子禁写 router TS，复跑
  聚焦 TS 测试确认唯一失败点后上报主 Agent，由 TS 可写节点/集成节点做
  一行断言同步。

不跑全量 `pnpm verify`；不操作 stable instance/Mongo/PM2/4004-4007。
`CARGO_TARGET_DIR=/Users/geek/workspace/wt-m-spawn-repair/target`。

## 交接

完成后向 `/root/router_rust_integration_b9` 报告 branch、worktree、
implementation commit/tree、实际写集、自验收矩阵、TS 影响面，并通知 root
（父 Agent）。
