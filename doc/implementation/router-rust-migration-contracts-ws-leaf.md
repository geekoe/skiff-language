# Router Rust Migration Batch 4 — contracts-ws Leaf Task

日期：2026-08-02
状态：execution leaf（一次性有界会话）
Agent：`/root/dev_contracts_ws`
集成目标：`/root/router_rust_integration_b4`

## 引用链

- 直接父批次：`doc/implementation/router-rust-migration-batch-4.md`
  （contract pack freeze，WS 链；baseline `main@7683b7c8`；该文档位于
  integration 分支 `f5032f0b`，main 基线不包含批次文档本身）。
- 权威设计：`doc/implementation/router-rust-migration-plan.md`（draft v5，
  2026-08-01），重点 §3.2（owner/invariant：`ClientConnectionIndex`、
  `RuntimeGenerationPinLedger`、`WebSocketRequestBroker`）、§3.4
  （`ClientSocketGeneration` 独立 newtype）、§3.7（client socket 独立
  finalization protocol，四向竞态）、§3.8（boundedness：single writer、
  frame/byte permit）、§5.3（C-model-connection lane）、§5.4（contract
  pack 必填项：owner/invariant、typed I/O、capacity、queue full、
  timeout/disconnect/replacement/shutdown terminal、health、fake seam、
  真实边界 probe）、§5.5（sink bundle 不含 client state）、§7（E-ws）。
  冲突时以权威设计为准。
- 本叶子落盘的冻结契约：
  - `doc/implementation/router-rust-migration-c-model-connection-contract.md`
  - `doc/implementation/router-rust-migration-c-client-lifecycle-contract.md`
  - `doc/implementation/router-rust-migration-c-ws-contract.md`

## 基线与环境

- 仓库：`/Users/geek/workspace/skiff`（主 worktree 在 integration 分支
  `f5032f0b`；本节点只读预检在该 HEAD 进行）。
- 精确 baseline：`main@7683b7c8007a374ae07cb62c7723ced62929100b`
  （`git rev-parse main` 已验证；本节点 worktree 精确锚定该 commit）。
- 分支 / worktree：`feat/router-rust-contracts-ws` /
  `/Users/geek/workspace/wt-contracts-ws`（基线即上述 commit）。
- `CARGO_TARGET_DIR=/Users/geek/workspace/wt-contracts-ws/target`
  （不与其他 worktree 共享）。

## 零 worktree 只读预检结论

1. baseline 锚定：`main` = `7683b7c8…`，与批次文档一致。
2. M0 后 transport 现状（`runtime/transport`）：
   - `connection_protocol.rs` 已有 `connection.request` /
     `connection.request.cancel` / `connection.response` 的严格 codec：
     profile `jsonrpc-2.0-text`、method ≤ 256B、payload ≤ 1 MiB、
     response outcome 集合（success/remote/deadlineExceeded/
     connectionUnavailable/transportUnavailable/protocolError/resourceLimit）、
     remote message ≤ 4096B、RFC3339 deadline。
   - `websocket_generation_lifecycle.rs` 已有 `websocket.generation.lifecycle`
     Acquire/Release/Ack/Reject 的严格 codec、方向/sender 校验、tuple
     identity 校验、response exact-echo 断言。
   - `cross-system-fixtures/package-service-ecosystem/websocket-generation-lifecycle-wire.json`
     与 `runtime/transport/src/websocket_generation_lifecycle/tests.rs`
     已冻结 lifecycle wire 的 JSON 形态。
   - 尚无 `ClientSocketGeneration` / `ClientConnectionIndex` /
     `RuntimeGenerationPinLedger` / `WebSocketRequestBroker` 类型
     （本 pack 冻结其契约与参考模型，不写 production 类型）。
3. TS 生产语义来源（只读锚定，未改动）：
   - `router/src/router/webSocketRequestBroker.ts`：peer/Runtime 双向
     correlation、deadline timer、tombstone FIFO/TTL、capacity、generation
     close/abort、captured writer 失败即 settle。
   - `router/src/router/webSocketGenerationLifecycleRouter.ts`：
     expect/acquire/release pending/cached acquire/release timeout（默认
     5s）/disconnect 清理/ACK 计数/flush 失败聚合。
   - `router/src/gateway/webSocketConnectionLifecycle.ts`：reserve/admit/
     attach、business key 索引、close-oldest / reject-new / ranked
     high-water replacement、single writer、slow-client budget（默认
     16 MiB）、observed write 计数、finalizer 恰好一次。
   - `router/src/gateway/webSocketRpcBridge.ts` 与
     `webSocketRpcConnectionAttachment.ts`：bridge 装配 broker generation、
     runtime receipt fence、finalize 链。
   - `router/src/protocol/jsonRpc20TextProfileImplementation.ts`：
     JSON-RPC 2.0 text 词法分类；numeric id 先按 lexeme 验证 safe
     integer 再 canonicalize（`1e0`→`1`、`-0`→`0`）；response id 仅接受
     非空字符串。
4. 现有 TS 测试（`router/tests/websocket-*.test.ts`）已覆盖上述语义；
   本节点冻结契约与 corpus，不改 TS 行为。

## 任务目标（contract pack：WS 链）

1. 冻结 C-model-connection：`connection_protocol` 现有 wire 的 byte-exact
   corpus（request/cancel/response + 负例）、`ClientSocketGeneration`
   身份 newtype、JSON-RPC 2.0 text numeric id 词法验证与 canonicalization
   corpus（`1e0`→`1`、`-0`→`0`、safe-integer 边界、response id 字符串
   规则）、profile 帧/字节预算。
2. 冻结 C-client-lifecycle：`ClientConnectionIndex` + business replacement
   （close-oldest / reject-new / ranked high-water）、`ClientSocketGeneration`
   finalization protocol（§3.7 四步）、single writer、frame/byte budget、
   slow-client saturation、四向竞态（replacement/peer close/runtime
   disconnect/shutdown）终态、release timeout 不静默保留 pin。
3. 冻结 C-ws：`RuntimeGenerationPinLedger`（acquire/release/pending/cache/
   session attachment）、`WebSocketRequestBroker`（peer correlation/
   deadline/tombstone/captured writer fence）、capacity、queue full、
   health fields、fake seam、真实边界 probe 定义。

## 交付清单

- 契约文档三份（见引用链）。
- Corpus fixture 与其测试（`runtime/transport/testdata/client-ws/` +
  `runtime/transport/tests/`）：
  - `client-ws/frames.json`：connection + websocket generation lifecycle
    帧目录（完整二进制 hex + typed header + decodeAs + direction）。
  - `client-ws/jsonrpc-ids.json`：JSON-RPC numeric/string id 词法 corpus。
  - `client-ws/scenarios/*.json`：client lifecycle + pin ledger + broker
    组成的参考状态机场景（含四向竞态与慢客户端终态）。
  - `tests/client_ws_corpus.rs`：byte-exact 帧校验 + id 词法校验 +
    场景参考状态机。
  - `tests/ws_generation_ledger_contract.rs`：pin ledger 参考模型
    （acquire/release/pending/cache/timeout/disconnect/flush）。
  - `tests/ws_broker_contract.rs`：broker 参考模型
    （correlation/deadline/tombstone/capacity/writer fence/close）。

## 写入边界

可写：

- `doc/implementation/router-rust-migration-contracts-ws-leaf.md` 及三份
  契约文档。
- `runtime/transport/testdata/client-ws/`（corpus fixtures）。
- `runtime/transport/tests/` 下三个测试文件（corpus/参考模型，test-only）。

禁止：

- `skiff-router` production、`runtime/transport/src` production、deployment/
  artifact-model production 类型、AGENTS.md、scripts README、verify
  注册表/selector graph/verify.yml、`skiff-instance.mjs`、
  `Cargo.toml`/`Cargo.lock`（本节点不需要新依赖）。
- 操作 stable instance、Mongo、PM2、4004-4007 端口进程；不跑全量
  `pnpm verify`。

## 自验收矩阵

| 验收项 | 命令 / 证据 |
| --- | --- |
| corpus 测试通过（帧/词法/场景） | `CARGO_TARGET_DIR=<worktree>/target cargo test --package skiff-runtime-transport --test client_ws_corpus --test ws_generation_ledger_contract --test ws_broker_contract` |
| 现有 transport 测试不回归 | `cargo test --package skiff-runtime-transport`（聚焦运行） |
| 契约文档覆盖 §5.4 必填项 | 三份文档均含 owner/invariant、typed inputs/outputs、capacity、queue full、timeout/disconnect/replacement/shutdown terminal、health fields、fake seam、real boundary probe |
| 无 production consumer 提前依赖 | `rg -n "contracts-ws|c-model-connection|c-client-lifecycle|c-ws|client-ws|RuntimeGenerationPinLedger|WebSocketRequestBroker" --glob '!doc/**' --glob '!runtime/transport/tests/**' --glob '!runtime/transport/testdata/**'`（无新增命中） |
| baseline/写集干净 | `git status` 仅上述新增文件；`git diff main...HEAD` 聚焦 |

## 交接

完成后向 `/root/router_rust_integration_b4` 报告 branch、worktree、
implementation commit/tree、实际写集、自验收矩阵，并通知 root（父 Agent）。
