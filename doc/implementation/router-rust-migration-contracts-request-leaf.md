# Router Rust Migration Batch 4 — contracts-request Leaf Task

日期：2026-08-02
状态：execution leaf（一次性有界会话）
Agent：`/root/dev_contracts_request`
集成目标：`/root/router_rust_integration_b4`

## 引用链

- 直接父批次：`doc/implementation/router-rust-migration-batch-4.md`
  （contracts-request 节点；baseline `main@7683b7c8`）。
- 权威设计：`doc/implementation/router-rust-migration-plan.md`（draft v5，
  2026-08-01），重点 §3.2（`RequestDispatcher`/`RuntimeAdmissionPool`
  owner 合同）、§3.3（active routing 单一 authority、candidate query、
  `RegisteredSessionLease`）、§3.4（identity/fence）、§3.6（session
  cancellation）、§3.8（boundedness）、§5.3（C-model-request →
  W-model-request → M-request）、§5.4（C-routing-query / C-dispatch 必填项
  与 pack 结构）、§5.5（demux/sink）、§7（E-dispatch/E-http）。冲突时以
  权威设计为准。
- 兄弟节点：`contracts-ws`、`contracts-actor`（并行，不写重叠文件）。
- 本叶子落盘的冻结契约：
  - `doc/implementation/router-rust-migration-c-model-request-contract.md`
  - `doc/implementation/router-rust-migration-c-routing-query-contract.md`
  - `doc/implementation/router-rust-migration-c-dispatch-contract.md`

## 基线与环境

- 仓库：`/Users/geek/workspace/skiff`。
- 精确 baseline：`main@7683b7c8`（`git rev-parse main` 已验证为
  `7683b7c8007a374ae07cb62c7723ced62929100b`；worktree 即该 commit）。
- 分支 / worktree：`feat/router-rust-contracts-request` /
  `/Users/geek/workspace/wt-contracts-request`。
- `CARGO_TARGET_DIR=/Users/geek/workspace/wt-contracts-request/target`
  （不与其他 worktree 共享）。

## 零 worktree 只读预检结论

1. baseline 锚定：`main` = `7683b7c8…`，与批次文档一致。
2. request wire 现状（`runtime/transport`，M0 拆分后）：
   - `runtime_assembly_request.rs`：`RuntimeAssemblyRequestStartFrameHeader`
     （HTTP unary/serverStream）、`RuntimeAssemblyRequestStartFrameWireHeader`
     （Http/WebSocketConnect/WebSocketJsonRpc/Spawn 四分支）、
     `decode_runtime_assembly_request_start_frame`（strict canonical JSON
     decode + typed validation + 变体 payload presence 规则）。
   - `protocol/request.rs`：`RequestCancelFrameHeader`（request.cancel）、
     `ResponseStartFrameHeader`、`ResponseChunkFrameHeader`、
     `ResponseEndFrameHeader`、`ResponseErrorFrameHeader`（fixedService/
     control）+ `validate_response_error_frame`。
   - `response_mapper.rs`：unary/stream 的 response.end phase 规则
     （payloadPresent ↔ payload；stream end 恒空）。
   - `cancel_reason.rs`：`RequestCancelReason`/`RequestCancelSituation`
     wire reason 契约表（CONTRACT_H 9 项）。
   - 真实 runtime consumer：`runtime/host/src/host/router_session.rs`
     decode `request.start`/`request.cancel`（payload 必须空）。
   - legacy `RequestStartFrameHeader`（envelope_type 形态）仍存在并被
     `request_mapper.rs` 消费；本 pack 冻结目标是 `runtime_assembly_request`
     形态，legacy 形态记录为差异（不删、不迁移）。
3. TS `router/src/router/runtimeDispatcher.ts` 语义（2581 行）：
   - `pending: Map<requestId, RuntimeInvocation>`，kind =
     unary | unaryFrame | websocketJsonRpc | derivedSpawn | stream；
   - admission：`pickDispatchConnection` + `assertConnectionAdmission`
     （per-connection in-flight < maxConcurrency）+ `assertRequestIdAvailable`；
   - stream 状态机：waitingStart → streaming → terminal；response.start 空
     payload、chunk seq 严格递增、response.end 空 payload/metadata；
   - terminal sources：runtime_response_end / runtime_response_error /
     runtime_request_cancel / timeout / caller_abort / client_disconnect /
     backpressure / protocol_error / callback_error / runtime_disconnect /
     router_shutdown；
   - cancel 帧只在 timeout/caller_abort/client_disconnect/backpressure/
     protocol_error/callback_error/router_shutdown 时发送（response 终态与
     runtime disconnect 不发送）；
   - function-spawn：`requireSpawnParent` 从 dispatcher pending（request
     parent）或 actor lane（`actorMethodSpawn`）二选一，同时命中即拒绝；
     `dispatchDerivedSpawn` 建立 kind=derivedSpawn 的 pending，response.end
     必须为空；actor-method spawn 由 `actorMethodSpawn.submitSpawn` 处理，
     不进 dispatcher pending。
4. 尚无 `RoutingEpoch`/`RegisteredSessionLease`/`RuntimeAdmissionPool`/
   `RequestDispatcher` production 类型（W-routing-query/W-dispatch 实现）；
   本 pack 只冻结契约与 corpus。

## 任务目标（contract pack：request 链）

1. 冻结 C-model-request：request wire（HTTP unary/serverStream
   `request.start`、`request.cancel`、`response.start`/`response.chunk`/
   `response.end`/`response.error`）的 exact JSON 形态、frame 级 direction、
   payload presence、stream 顺序与终态语义、cancel reason 词表；byte-exact
   corpus。
2. 冻结 C-routing-query：captured `RoutingEpoch` + directory exact
   registered tuple/registration revision + cancellation → exact
   `RuntimeSessionEpoch` candidates（`RegisteredSessionLease`）；heartbeat
   不参与 admission；每次查询只读一个完整 revision。
3. 冻结 C-dispatch：routing epoch capture → candidate query → reserve
   permit → enqueue 前原子 revalidate → enqueue → terminal 恰好释放一次；
   `RequestDispatcher` pending/terminal/function-spawn correlation
   （actor-method spawn 归 actor lane）；queue full / timeout / disconnect /
   replacement / shutdown terminal；health fields；fake seam；真实边界 probe。

## 交付清单

- 契约文档三份（见引用链）。
- Corpus fixture 与其测试（`runtime/transport/testdata/` +
  `runtime/transport/tests/`）：
  - `request-wire/frames.json`（byte-exact 帧目录：request.start unary/
    serverStream、request.cancel、response.start/chunk/end（payload + stream
    empty）、response.error control/fixedService；每帧完整 hex + header +
    decodeAs + direction + payload 规则）。
  - `request-wire/reject-cases.json`（codec 级负例 JSON header）。
  - `request-wire/scenarios/*.json`（unary/stream 合法序列与
    wrong-order/seq/payload 违规、cancel 双向、response.error 终态）。
  - `routing-query/scenarios/*.json`（exact candidate、多 replica、cancelled
    排除、revision/tuple/capability 不匹配、heartbeat 不参与）。
  - `dispatch-admission/scenarios/*.json`（admission 全流水线、queue full、
    revalidate fail + reselect、cursor、全部 terminal、function-spawn /
    actor-method spawn 归属、替换/shutdown）。
  - `tests/request_wire_corpus.rs`、`tests/routing_query_corpus.rs`、
    `tests/dispatch_admission_corpus.rs`（参考模型 + byte-exact/codec 断言）。

## 写入边界

可写：

- `doc/implementation/router-rust-migration-contracts-request-leaf.md` 及三份
  契约文档。
- `runtime/transport/testdata/request-wire/`、
  `runtime/transport/testdata/routing-query/`、
  `runtime/transport/testdata/dispatch-admission/`（corpus fixtures）。
- `runtime/transport/tests/request_wire_corpus.rs`、
  `runtime/transport/tests/routing_query_corpus.rs`、
  `runtime/transport/tests/dispatch_admission_corpus.rs`（test-only）。

禁止：

- `router/`、`runtime/transport/src`、`deployment/` 任何 production 修改；
- AGENTS.md、scripts README、verify 注册表/selector graph/verify.yml、
  `scripts/skiff-instance.mjs`；
- `Cargo.toml`/`Cargo.lock`（本节点不需要新依赖）；
- 操作 stable instance、Mongo、PM2、4004-4007 端口进程；不跑全量
  `pnpm verify`；不跑 chat smoke（不涉及 Agine 链路）。

## 自验收矩阵

| 验收项 | 命令 / 证据 |
| --- | --- |
| corpus 测试通过（含负例） | `CARGO_TARGET_DIR=<worktree>/target cargo test --package skiff-runtime-transport --test request_wire_corpus --test routing_query_corpus --test dispatch_admission_corpus` |
| 现有 transport 测试不回归 | `cargo test --package skiff-runtime-transport`（聚焦运行） |
| 契约文档覆盖 §5.4 必填项 | 三份文档均含 owner/invariant、typed inputs/outputs、capacity、queue full、timeout/disconnect/replacement/shutdown terminal、health fields、fake seam、real boundary probe |
| 无 production consumer 提前依赖 | `rg -n "contracts-request|c-model-request|c-routing-query|c-dispatch|request-wire|routing-query|dispatch-admission|RegisteredSessionLease" --glob '!doc/**' --glob '!runtime/transport/tests/**' --glob '!runtime/transport/testdata/**'`（无命中） |
| baseline/写集干净 | `git status` 仅上述新增文件；`git diff main...HEAD` 聚焦 |

## 交接

完成后向 `/root/router_rust_integration_b4` 报告 branch、worktree、提交
hash、corpus 测试命令与结果、rg 反向搜索证据；同步通知 root。
