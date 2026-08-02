# Router Rust Migration Batch 6 — W-model-request / M-request Leaf Task

日期：2026-08-02
状态：execution leaf（一次性有界会话）
Agent：`/root/dev_w_model_request`
集成目标：`/root/router_rust_integration_b6`

## 引用链

- 批次文档：`doc/implementation/router-rust-migration-batch-6.md`
  （W-model-request + M-request 节点；baseline `main@8cabf352`）。
- 权威设计：`doc/implementation/router-rust-migration-plan.md`（draft v5），
  重点 §3.2（`RequestDispatcher` 与 ordinary unary/stream）、§3.8
  （boundedness、业务 payload 为 immutable opaque bytes）、§5.3
  （C-model-request → W-model-request → M-request）、§5.4
  （C-dispatch + M-request）、§5.5（Request family sink）。冲突时以权威
  设计为准。
- 冻结契约：`doc/implementation/router-rust-migration-c-model-request-contract.md`
  （corpus：`runtime/transport/testdata/request-wire/`；§9 为
  W-model-request 交付义务）。
- 同链契约：`doc/implementation/router-rust-migration-c-dispatch-contract.md`
  （unknown cancel reason 的 pending terminal 语义）、
  `doc/implementation/router-rust-migration-c-routing-query-contract.md`。
- 先例叶子：`doc/implementation/router-rust-migration-w-model-leaf.md`
  （W-model-registration / W-model-bootstrap-wire / M 双 consumer gate 的
  corpus 消费模式）。

## 基线与环境

- 仓库：`/Users/geek/workspace/skiff`。
- 精确 baseline：`main@8cabf352`（`git rev-parse main` 已验证；
  worktree HEAD 与 main 一致）。
- 分支 / worktree：`feat/router-rust-w-model-request` /
  `/Users/geek/workspace/wt-w-model-request`。
- `CARGO_TARGET_DIR=/Users/geek/workspace/wt-w-model-request/target`
  （不与其他 worktree 共享）。

## 零 worktree 只读预检结论

1. baseline 锚定：`main` = `8cabf352…`，与批次文档一致；当前 checkout 的
   `integration/router-rust-migration-batch-6` 分支比 main 仅多批次父文档
   commit（`23ddab00 docs(router-rust): add batch 6 execution parent`）。
2. request wire 现状（`runtime/transport/src`，M0 拆分后）：
   - `runtime_assembly_request.rs`：`RuntimeAssemblyRequestStartFrameHeader`
     （HTTP unary/serverStream）、`RuntimeAssemblyRequestStartFrameWireHeader`
     （Http/WebSocketConnect/WebSocketJsonRpc/Spawn 四分支）、
     `decode_runtime_assembly_request_start_frame`（strict canonical JSON
     decode + typed validation + 变体 payload presence 规则）。
   - `protocol/request.rs`：`RequestCancelFrameHeader`、
     `ResponseStartFrameHeader`、`ResponseChunkFrameHeader`、
     `ResponseEndFrameHeader`（payloadPresent + optional metadata）、
     `ResponseErrorFrameHeader`（fixedService/control）+ 
     `validate_response_error_frame`（变体 payload 规则已强制）。
   - `response_mapper.rs`：`validate_response_end_frame(header, payload,
     phase)` 的 phase 校验（Payload/Http 两相，`payloadPresent ==
     !payload.is_empty()`）；unary/stream 的 encode 侧既有函数。
   - `cancel_reason.rs`：`RequestCancelReason`（12 项）+ `CONTRACT_H`
     （9 项 wire reason 词表）。
   - 真实 runtime consumer（`runtime/host/src/host/router_session.rs`）对
     `request.cancel` 自己强制空 payload；transport codec 层尚未强制
     （契约 §7 差异记录：W-model-request 翻转 `currentEnforced`）。
   - legacy `RequestStartFrameHeader`（envelope_type 形态）仍存在并被
     `request_mapper.rs` 消费；本节点不删、不迁移。
3. corpus 现状：
   - `runtime/transport/testdata/request-wire/frames.json`（12 帧，
     byte-exact hex + direction + frameType + decodeAs + payloadRule +
     payloadHex + header）。
   - `runtime/transport/testdata/request-wire/reject-cases.json`（10 个
     request.start codec 级负例）。
   - `runtime/transport/testdata/request-wire/scenarios/*.json`（13 个
     序列语义场景）。
   - `runtime/transport/tests/request_wire_corpus.rs`（contracts-request
     交付：reference wire 状态机 + 真实 codec byte-exact roundtrip；
     本节点不改写其语义）。
   - 共享 corpus `cross-system-fixtures/package-service-ecosystem/
     runtime-request-wire.json` 存在；本节点不改变其字节。
4. 依赖坐标（设计空洞检查）：
   - 冻结契约 §2 规定 request.cancel / response.start / response.chunk /
     response.end 的 canonical codec 为 `decode_typed_binary_frame::<…>`；
     W-model 在其上叠加 payload-presence / payloadPresent 一致性 /
     cancel reason 词表强制，不替换既有 typed DTO。
   - `response.end` 的 stream 顺序与终态语义（waitingStart/streaming/
     terminal、chunk seq、stale）归 C-dispatch / dispatcher 状态机
     （C-model-request §5.4、C-dispatch §4.1）；wire codec 只强制
     `payloadPresent == !payload.is_empty()` 一致性，不做 mode 决策。
   - frame 级 direction/payload presence 强制在 transport 提供
     `request_frame_rule` 分类表面；`RuntimeFrameDemux` 装配归
     W-dispatch（本节点不写 `router/src/`）。当前 demux 用 family
     wire_type_prefix `request.` 匹配，`response.*` 帧不命中 Request
     family，是 W-dispatch 接线时需消费 `request_frame_rule` 的已知缝隙。
   - 不新增 workspace crate，不改 `Cargo.toml` / `Cargo.lock`
     （router/runtime 已依赖 transport）。

## 任务范围

1. W-model-request：在 transport 实现/收敛 request family 的 frame 级
   DTO/codec：
   - `request.cancel` codec（payload-empty 强制，翻转 `currentEnforced`；
     reason 必须是 `RequestCancelReason::CONTRACT_H` 9 项词表，unknown
     reason 拒绝帧）；
   - `response.start` codec（payload-empty 强制）；
   - `response.chunk` codec（payload optional，可为空）；
   - `response.end` codec（`payloadPresent == !payload.is_empty()` 一致性
     强制；serverStream 空终态由 dispatcher 按 mode 校验）；
   - `response.error` 沿用既有 `decode_response_error_frame`（变体 payload
     规则已强制）；
   - `request.start` 沿用既有 `decode_runtime_assembly_request_start_frame`
     （strict canonical；本节点不改变其既有单元测试语义）。
2. frame 级 direction/payload presence 分类表面
   （`RequestFrameKind` / `RequestFrameRule` / `request_frame_rule`），
   供 `RuntimeFrameDemux`/sink 装配（W-dispatch 接线）。
3. M-request Rust consumer gate：skiff-router 与 runtime crate 的 consumer
   测试直接消费同一 corpus（`runtime/transport/testdata/request-wire/`），
   不复制 fixture。
4. 交付叶子任务文件（本文件）。

非目标：不实现 W-dispatch 的 pending/terminal/admission 状态机；不写
`router/src/` production；不写 `runtime/host` production；不删 legacy
`RequestStartFrameHeader`；不改 contracts-request 已冻结的
frames.json / reject-cases.json / scenarios / request_wire_corpus.rs 语义；
不改 cross-system corpus 字节。

## 写集（全部在 worktree `/Users/geek/workspace/wt-w-model-request`）

production（`runtime/transport/src`，仅 request 模块 owner）：

1. `src/protocol/request.rs`：新增 request.cancel / response.start /
   response.chunk / response.end 的 frame 级 codec
   （`decode_*` / `encode_*` / `validate_*`）与
   `RequestFrameKind` / `RequestFramePayloadPresence` / `RequestFrameRule` /
   `request_frame_rule` 分类表面。
2. `src/cancel_reason.rs`：新增 `RequestCancelReason::is_contract_h()` /
   `from_contract_h_wire()`（供 cancel codec 词表强制）。
3. `src/protocol.rs`：re-export 新增表面（registry 最小改动）。

corpus / tests（`runtime/transport`，W-model owner）：

4. `tests/w_model_request_corpus.rs`：新 corpus 测试（w_model_* 前缀）：
   frames.json 全部帧经 W-model codec byte-exact roundtrip + payloadHex
   一致；reject-cases.json 全部负例 fail closed；新增强制负例
   （cancel 非空 payload、unknown/non-CONTRACT_H reason、response.start
   非空 payload、response.end payloadPresent/payload 不一致）；
   `request_frame_rule` 分类断言。

consumer gates：

5. `router/tests/w_model_request_consumer.rs`（skiff-router consumer：
   encode request.start / request.cancel，decode response.* 全部帧，
   byte-exact roundtrip）。
6. `runtime/tests/w_model_request_consumer.rs`（runtime crate consumer：
   decode request.start / request.cancel，encode response.* 全部帧，
   byte-exact roundtrip）。

doc：

7. `doc/implementation/router-rust-migration-w-model-request-leaf.md`
   （本文件）。

禁止写：`router/src/`、`runtime/host`、deployment、artifact-model
production、verify 注册表 / selector graph / verify.yml、AGENTS.md、
scripts README、`scripts/skiff-instance.mjs`、`Cargo.toml` / `Cargo.lock`、
`runtime/transport/src` 非 request 模块（session/activation/connection/
actor/spawn/assembly_activation/ingress_selector/…）、
`runtime/transport/testdata/request-wire/` 既有 fixture。

## 自验收矩阵

| 项 | 证据 |
| --- | --- |
| transport corpus 测试 | `CARGO_TARGET_DIR=<worktree>/target cargo test -p skiff-runtime-transport --test w_model_request_corpus` |
| transport 既有测试不回归 | `cargo test -p skiff-runtime-transport`（含 request_wire_corpus 与全部 unit tests） |
| router consumer gate | `cargo test -p skiff-router --test w_model_request_consumer` |
| runtime consumer gate | `cargo test -p runtime --test w_model_request_consumer` |
| golden bytes 不变 | corpus 测试断言 `encode(decode(hex)) == hex`；git diff 审计不触碰 request-wire fixtures 与 cross-system fixture |
| 强制已生效 | w_model_request_corpus 断言 cancel 非空 payload / unknown reason、response.start 非空 payload、response.end payloadPresent 不一致均被真实 codec 拒绝 |
| frame 级分类 | `request_frame_rule` 对 6 个 frame type 返回冻结 direction/presence；unknown type 返回 None |
| 写集干净 | `git status` 仅本叶子写集；`git diff main...HEAD` 聚焦 |

不跑全量 `pnpm verify`；不操作 stable instance/Mongo/PM2/4004-4007；
不跑 chat smoke（不涉及 Agine 链路）。

## 交接

完成后向 `/root/router_rust_integration_b6` 报告 branch、worktree、
implementation commit/tree、实际写集、自验收矩阵；同步通知 root。
