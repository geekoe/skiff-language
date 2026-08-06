# Router Rust Migration Batch 5 — W-model-activation + W-model-connection Leaf

日期：2026-08-02
状态：execution leaf（一次性有界会话）
Agent：`/root/dev_w_model_activation_connection`
集成目标：`/root/router_rust_integration_b5`

> 已由 M4 取代（2026-08-06，runtime-lazy-deploy）：本文所述 activation 协调层 / epoch 部署机制
> （`assembly.activation` 帧族、`/__skiff/activate-assembly`、activation 状态仓库与配置项、
> committed/expected generation 术语等）已在 M4 全部下线，部署语义以
> [`doc/architecture/runtime-lazy-load-deployment.md`](../../architecture/runtime-lazy-load-deployment.md)
> 为准。本文保留为历史执行记录。

## 引用链

- 批次文档：`doc/implementation/router-rust-migration-batch-5.md`
  （W-model-activation-connection 节点、写边界、验证 owner、退出检查点）。
- 权威设计：`doc/implementation/router-rust-migration-plan.md`（draft v5）
  - §4.1 live transaction / §4.2 cold recovery（activation transaction wire）；
  - §5.3 C-model-activation → W-model-activation → M-activation、
    C-model-connection → W-model-connection → M-connection；
  - §5.4 contract pack 必填项；§5.5 sink/demux；§7 E-ws。
- 冻结契约：
  - `doc/implementation/router-rust-migration-c-model-activation.md`
    （事务 wire 五变体、方向矩阵、stale ACK 拒绝与 participant binding、
    §9 验证映射）；
  - `doc/implementation/router-rust-migration-c-activation-coordinator.md`；
  - `doc/implementation/router-rust-migration-c-model-connection-contract.md`
    （connection wire、`ClientSocketGeneration`、JSON-RPC 2.0 text 词法
    契约与 numeric id canonicalize、§6.2 typed I/O：`OpaquePeerId` /
    `ProfileAction`）；
  - `doc/implementation/router-rust-migration-c-client-lifecycle-contract.md`、
    `doc/implementation/router-rust-migration-c-ws-contract.md`。
- 父叶子：`router-rust-migration-contracts-activation-leaf.md`、
  `router-rust-migration-contracts-ws-leaf.md`（corpus 交付与 location 决策）。
- 同批次兄弟节点：W-bootstrap、H-registration-cut、W-model-actor-spawn
  （文件前缀/ownership 划分见批次文档）。

## 基线与环境

- 仓库：`/Users/geek/workspace/skiff`。
- 精确 baseline：`main@85596193df24f1fb5d0745eabf049e7e1ebf5a79`
  （`git rev-parse 85596193` 已验证；worktree HEAD 相同）。
- Worktree：`/Users/geek/workspace/wt-w-model-activation-connection`，
  branch `feat/router-rust-w-model-activation-connection`。
- `CARGO_TARGET_DIR=/Users/geek/workspace/wt-w-model-activation-connection/target`
  （不与其他 worktree 共享；兄弟节点 `wt-w-model-actor-spawn` 并行存在，
  文件集合与本节点无重叠）。

## 零 worktree 只读预检结论

1. baseline 锚定成功；主 worktree 当前在
   `integration/router-rust-migration-batch-5`（966d8164，仅比 main 多批次文档）。
2. transport 现状（`runtime/transport/src`）：
   - `assembly_activation.rs`：activation family 帧级 codec 已完整
     （schemaVersion/type/payload-empty/direction 矩阵严格校验），DTO
     `AssemblyActivationControl`（artifact-model）已按 C-model-activation §2
     冻结；`expectedGeneration + 1 == candidateGeneration`、token、
     strict refs、serviceDb 仅 Prepare/Commit 均已有实现。
   - `connection_protocol.rs`：connection.request / cancel / response 帧级
     codec 已完整（profile、deadline、payload 组合规则、remote 约束）。
   - `websocket_generation_lifecycle.rs`：Acquire/Release/Ack/Reject codec
     已完整（direction/sender/tuple/response exact-echo）。
   - 缺失（本叶子 W-model-connection 生产收敛点）：
     `ClientSocketGeneration` newtype、`OpaquePeerId` / `ProfileAction`、
     JSON-RPC 2.0 text 词法分类器（C-model-connection §2/§5/§6 的 typed I/O
     输出目前只有 test-only 参考实现，`client_ws_corpus.rs` 明确标注非
     production）。
3. corpus 现状：
   - activation 帧与事务 corpus 在 `cross-system-fixtures/package-service-ecosystem/`
     （`control-wire.json` / `runtime-wire.json` golden bytes +
     `activation-transaction-cases.json` 22 cases：live 16 / coldRecovery 6），
     由 `runtime/transport/src/assembly_activation/tests.rs` 与
     `runtime/transport/tests/activation_transaction_corpus.rs` 消费。
     **不存在 `runtime/transport/testdata/activation` 目录**；父节点路径
     猜测不准确。本叶子不复制/迁移共享 fixture（避免字节漂移），consumer
     gate 直接引用 cross-system 共享 corpus（满足“直接消费同一 corpus”）。
   - connection corpus 在 `runtime/transport/testdata/client-ws/`
     （`frames.json` 17 帧 byte-exact、`jsonrpc-ids.json` 22 cases、
     `scenarios/*.json` 23 场景），由 `client_ws_corpus.rs`（test-only
     参考模型）+ `ws_generation_ledger_contract.rs` + `ws_broker_contract.rs`
     消费。
4. consumer gate 先例：batch 4 W-model 已交付
   `runtime/tests/w_model_registration_consumer.rs` /
   `w_model_bootstrap_wire_consumer.rs` 与 `router/tests/` 同名文件；
   本叶子沿用同一模式与 corpus 路径解析。
5. 依赖检查：`skiff-router` 与 `runtime` crate 均已依赖
   `skiff-runtime-transport` 且 dev 依赖含 serde_json；不需要改
   `Cargo.toml`/`Cargo.lock`。`RuntimeSessionEpoch` 属于 skiff-router
   session identity（W-session 交付），本叶子不写 router/src，因此
   `ActivationParticipantBinding { replica_id, session: RuntimeSessionEpoch }`
   保持 coordinator 内部类型（C-model-activation §3 明确 session epoch 不
   上 wire、本契约不定义 production 类型），由 W-activation/E-activation
   在 router/src 实现；transport 侧无重复类型。
6. 无设计空洞：任务可在冻结契约与共享 corpus 上闭合，不需要扩 scope。

## 任务范围

1. W-model-activation：确认/收敛 activation transaction wire（五变体 +
   Register 同 family 边界）与生产 codec；新增 transport 级 W-model corpus
   gate（`w_model_activation_corpus.rs`）直接消费 cross-system 共享 corpus，
   断言 golden bytes 逐字节一致、方向矩阵、mutation 负例、事务 cases 完整。
2. W-model-connection：在 `connection_protocol.rs` 生产实现
   `ClientSocketGeneration`、`OpaquePeerId`、`ProfileAction` 与
   `classify_jsonrpc_20_text_frame`（C-model-connection §5/§6 typed I/O）；
   新增 `w_model_connection_corpus.rs` 用生产分类器消费
   `jsonrpc-ids.json` 全部 22 cases，并用生产 codec 消费 `frames.json` 17 帧。
3. M-activation / M-connection Rust consumer gate：`runtime` crate 与
   `skiff-router` 的 consumer 测试（`w_model_activation_*` /
   `w_model_connection_*` 前缀）直接消费同一 corpus，不复制 fixture。
4. 交付叶子任务文件（本文件）。

非目标：不实现 coordinator/ledger/broker/client-index 状态机
（W-activation / W-WebSocket / E-*）；不写 skiff-router production
（`router/src/`）；不写 `runtime/host` / deployment / artifact-model
production；不写 actor/spawn 模块；不删除 legacy wire；不改 cross-system
corpus bytes。

## 写集（全部在 worktree `/Users/geek/workspace/wt-w-model-activation-connection`）

production（`runtime/transport/src`，仅 connection 模块，本叶子 owner）：

1. `src/connection_protocol.rs`：新增 `ClientSocketGeneration`、
   `OpaquePeerId`、`JsonRpcPlatformErrorKind`、`ProfileAction`、
   `classify_jsonrpc_20_text_frame` 与 profile 预算常量（C-model-connection
   §5/§6.2）；复用现有 connection wire codec，不改既有字节语义。
2. `src/tests/connection_protocol.rs`：新增 identity/classifier 单元测试
   （canonicalization、parse/invalidRequest/invalidParams、close 1002/1009、
   预算负例）。

corpus / tests（`runtime/transport`）：

3. `tests/w_model_activation_corpus.rs`（新）。
4. `tests/w_model_connection_corpus.rs`（新）。

consumer gates：

5. `runtime/tests/w_model_activation_consumer.rs`（新）。
6. `runtime/tests/w_model_connection_consumer.rs`（新）。
7. `router/tests/w_model_activation_consumer.rs`（新）。
8. `router/tests/w_model_connection_consumer.rs`（新）。

doc：

9. `doc/implementation/router-rust-migration-w-model-activation-connection-leaf.md`
   （本文件）。

禁止写：skiff-router production（`router/src/`）、`runtime/host` production、
deployment、artifact-model production、`runtime/transport/src` 的
actor/spawn 模块与 `protocol.rs`/`lib.rs`（本节点无新增 re-export 需求）、
verify 注册表 / selector graph / verify.yml、AGENTS.md、scripts README、
`scripts/skiff-instance.mjs`、`Cargo.toml` / `Cargo.lock`。

## 自验收矩阵

| 项 | 证据 |
| --- | --- |
| transport W-model corpus gate | `cargo test -p skiff-runtime-transport --test w_model_activation_corpus --test w_model_connection_corpus` |
| transport 既有 corpus 不回归 | `cargo test -p skiff-runtime-transport`（含 activation_transaction / client_ws / ws_* / 全部 unit tests） |
| router consumer gate | `cargo test -p skiff-router --test w_model_activation_consumer --test w_model_connection_consumer` |
| runtime consumer gate | `cargo test -p runtime --test w_model_activation_consumer --test w_model_connection_consumer` |
| golden bytes 逐字节一致 | 各 gate 断言 `encode(decode(frameHex)) == frameHex`；`git diff` 审计不触碰既有 corpus fixture 与 cross-system fixture |
| JSON-RPC 词法 corpus | 生产 `classify_jsonrpc_20_text_frame` 消费 `jsonrpc-ids.json` 全部 cases（`1e0`→`1`、`-0`→`0`、safe-integer 边界、parse/invalidRequest、response numeric id → close 1002） |
| 无 production 提前依赖 | `rg` 反向搜索：`ClientSocketGeneration`/`OpaquePeerId`/`ProfileAction`/`classify_jsonrpc_20_text_frame` 新引用仅存在于 transport connection 模块、tests/ 与 doc/ |
| 写集干净 | `git status` 仅本叶子写集；`git diff main...HEAD` 聚焦 |

不跑全量 `pnpm verify`；不操作 stable instance/Mongo/PM2/4004-4007。

## 交接

完成后向 `/root/router_rust_integration_b5` 报告 branch、worktree、
implementation commit/tree、实际写集、自验收矩阵，并通知 root（父 Agent）。

## 执行结果（提交前自验收）

（2026-08-02 提交前填写，全部通过）

1. `cargo test -p skiff-runtime-transport`：lib 124 passed（含新增
   ClientSocketGeneration / JSON-RPC classifier 单元测试），全部集成
   test binary passed，其中 `w_model_activation_corpus` 3 passed、
   `w_model_connection_corpus` 3 passed；既有
   `activation_transaction_corpus` / `client_ws_corpus` /
   `ws_generation_ledger_contract` / `ws_broker_contract` 无回归。
2. `cargo test -p skiff-router --test w_model_activation_consumer --test
   w_model_connection_consumer --test session_handshake_corpus`：2+2+1
   passed，router consumer gate 与 W-session corpus 无回归。
3. `cargo test -p runtime --test w_model_activation_consumer --test
   w_model_connection_consumer`：2+2 passed。
4. golden bytes：activation 五变体（prepare/prepared/reject/commit/abort）
   与 connection 17 帧经生产 codec 均断言
   `encode(decode(frameHex)) == frameHex` 且 header JSON 与 corpus 逐字段
   一致；reverse direction encode/decode 全部 fail closed；
   `jsonrpc-ids.json` 22 cases 全部经生产
   `classify_jsonrpc_20_text_frame` 消费（`1e0`→`1`、`-0`→`0`、
   safe-integer 边界、parse/invalidRequest、response numeric id → close
   1002）。
5. rustfmt：8 个触碰 Rust 文件 `rustfmt --edition 2021` 通过；workspace
   `cargo fmt --all --check` 剩余差异全部为 baseline 既有未触碰文件
   （deployment/src/lib.rs、runtime/eval/src/actor_executor/tests.rs、
   transport tests 中 actor/ws/spawn 既有 corpus 文件），本节点写集零
   格式差异。
6. clippy：`cargo clippy -p skiff-runtime-transport -p skiff-router
   --tests` 与 `cargo clippy -p runtime --tests` 对本节点新增代码零
   warning/error；剩余 warning 均为 baseline 既有（connection_protocol
   既有 3 处、deployment/artifact-model 等未触碰 crate）。
7. 反向搜索：`ClientSocketGeneration` / `OpaquePeerId` / `ProfileAction` /
   `classify_jsonrpc_20_text_frame` 在 Rust 侧只出现在本叶子写集
   （transport connection 模块 + tests/）；TS 侧命中为 baseline 既有
   profile 实现/契约，未改动。
8. 写集：仅本叶子 9 个文件（2 修改 + 7 新增）；既有 corpus fixture
   （`client-ws/`、`registration-handshake/`、cross-system fixtures）零
   改动。
