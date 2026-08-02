# Router Rust Migration Batch 6 — W-http Leaf Task

日期：2026-08-02
状态：execution leaf（一次性有界会话）
Agent：`/root/dev_w_http`
集成目标：`/root/router_rust_integration_b6`

## 引用链

- 直接父批次：`doc/implementation/router-rust-migration-batch-6.md`
  （W-http 节点；baseline `main@8cabf352`；批次文档在
  `integration/router-rust-migration-batch-6` 分支，本 worktree 锚定 main）。
- 权威设计：`doc/implementation/router-rust-migration-plan.md`（draft v5），
  重点 §5.4（W-http / C-dispatch pack）、§7（E-http）、§8
  （`router-rust-http-live` 定义，W-http 只做 real HTTP → fake dispatcher，
  E-http 再接 real Runtime）。
- 冻结机制：`doc/implementation/router-rust-migration-c-net-contract.md`
  （hyper 1 + tokio multi-thread + Semaphore cap + watch/drain/abort；
  C-net 只冻结 listener 机制，不冻结 HTTP 业务端口/path）。
- 冻结契约：
  - `router-rust-migration-c-model-request-contract.md`（request wire
    DTO/codec/corpus：request.start HTTP unary/serverStream、cancel reason
    词表、response.start/chunk/end/error、stream 顺序）。
  - `router-rust-migration-c-dispatch-contract.md`（admission/pending/
    terminal 契约；dispatch port 输入 `DispatchRequest`；fake seam；同 wave
    以契约为准，W-http 不依赖 W-dispatch 实现）。
  - `router-rust-migration-c-routing-query-contract.md`（captured epoch /
    exact tuple / capability 语义，HTTP ingress 解析消费同一 projection
    精神）。

## 基线与环境

- 仓库：`/Users/geek/workspace/skiff`。
- 精确 baseline：`main@8cabf352`（worktree HEAD 已验证为
  `8cabf35289e87a610c0940b6aa10af3a0e67d64e`）。
- 分支 / worktree：`feat/router-rust-w-http` /
  `/Users/geek/workspace/wt-w-http`。
- `CARGO_TARGET_DIR=/Users/geek/workspace/wt-w-http/target`
  （不与其他 worktree 共享）。

## 零 worktree 只读预检结论

1. baseline 锚定：`main` = `origin/main` = `8cabf352…`；批次 6 执行父文档
   在 `integration/router-rust-migration-batch-6@23ddab00`，本 worktree
   按批次要求锚定 main。
2. C-net 机制已由 PR 0b 装配进 `router/src/listener.rs`（public HTTP 空响应、
   runtime/control WS、Semaphore cap、watch + drain deadline + abort）。
   W-http **不修改** listener/run_router/main（E-bootstrap gate 拥有），
   独立提供 `router/src/http/` 的 HTTP socket 层与真实 socket 探针。
3. `RoutingEpoch`（`router/src/bootstrap/epoch.rs`）当前只暴露
   `RuntimeAssembly.gateway_ingress`（selector/deployment/gatewayEntryKey/
   gatewayEntryIdentity）与 deployment projection；**epoch 投影不含
   HTTP surface（operation mode / adapter kind）**——TS 侧由
   `runtimeAssemblyDeploymentSnapshot.ts` 从 deployment gateway entries
   富化。W-http 在本模块内定义 typed `HttpGatewaySurfaceView`
   （gateway_entry_key → mode/adapter_kind），由
   `skiff-artifact-model::DeploymentGatewayEntry` 转换；`EpochHttpIngressResolver`
   消费 captured `Arc<RoutingEpoch>` + surface view 完成 exact 匹配。
   该 seam 写入本 leaf，E-bootstrap/E-http 接线时把 surface view 提升到
   composition（不阻塞 W-http 的 real HTTP → fake dispatcher 边界）。
4. request wire canonical DTO/codec 在 `skiff-runtime-transport`
   （`RuntimeAssemblyRequestStartFrameHeader`、`encode_binary_frame`、
   `decode_typed_binary_frame`、`decode_response_error_frame`、
   `RequestCancelReason`）；W-http 直接消费，不复制 codec。
5. TS `assemblyHttpGateway.ts` / `httpCors.ts` / `httpStreamResponseWriter.ts`
   / `serviceDeploymentSelection.ts` / `errors.ts` 是当前语义基线：
   trusted selector headers（X-Skiff-Service / X-Skiff-Version /
   X-Skiff-Release 冲突检查）、origin-form URL、CORS preflight /
   service-managed、request body limit、cumulative response ceiling、
   backpressure drain timeout、client disconnect cancel、test-case
   correlation header 隔离。
6. 不操作 stable instance / Mongo / PM2 / 4004-4007；不跑全量 `pnpm verify`；
   不跑 chat smoke（不涉及 Agine 链路）。

## 任务目标（W-http：HTTP socket 层，real HTTP → fake dispatcher）

1. `router/src/http/`：HTTP socket 层（C-net 冻结机制、独立于 run_router）：
   - trusted selector：`X-Skiff-Service` / `X-Skiff-Version`
     （`X-Skiff-Release` 兼容与冲突拒绝），singular/canonical token 校验；
   - service-scoped ingress：captured `Arc<RoutingEpoch>` +
     `HttpGatewaySurfaceView` → exact `HttpIngressBinding`
     （deployment + gatewayEntryIdentity + mode + adapterKind）；
     OPTIONS 显式 binding 决定 CORS service-managed；
   - typed/raw opaque payload：`request.start` typed header + raw body bytes
     （optional opaque payload；body limit → 413 `RequestTooLarge`）；
   - unary/stream mapping：unary `response.end` HTTP phase；stream
     response.start → chunks → empty end；chunk 顺序由 writer 串行队列保证；
   - stream sequencing：chunk-before-start / end-before-start /
     end-with-payload 在 HTTP 映射层 fail closed（`callback_error` terminal）；
     seq 参考状态机由 fake dispatcher 依 corpus 强制；
   - cumulative response ceiling：`http_max_response_bytes`（unary payload
     与 stream 累计）→ 502 `ResponseTooLarge`；
   - backpressure：bounded stream channel + drain timeout → terminal
     `backpressure` + cancel frame `backpressure`；
   - disconnect/cancel/deadline：client disconnect → cancel frame
     `client_disconnect`；dispatch deadline → 504 `TimeoutError` + cancel
     frame `timeout`；runtime `request.cancel`/`response.error`
     （control/fixedService）映射到平台 error；
   - CORS preflight / service-managed / platform error：自动 CORS headers +
     204 preflight；显式 OPTIONS ingress 时 service-managed；
     error body `{ "error": { code, message, details } }`；
   - test-dispatch isolation：`x-skiff-test-case-capability` /
     `x-skiff-test-case-parent-request-id` 进入 `testEffectsEnabled` /
     `testCaseCapability` / `testCaseParentRequestId`，且从
     `httpRequest.headers` 中剥离；自动 preflight 携带 correlation 头拒绝。
2. `router/src/lib.rs`：additive `pub mod http;` + 关键类型 re-export。
3. `router/tests/http_*`：真实 socket HTTP → fake dispatcher 探针
   （unary/stream/CORS/ceiling/backpressure/disconnect/deadline/selector/
   test-dispatch isolation）。
4. 相关 doc：本 leaf。

## 写入边界

可写：

- `router/src/http/`（本节点独占）。
- `router/src/lib.rs`（仅 additive：模块声明与 re-export）。
- `router/tests/http_*.rs`（真实 socket 探针）。
- `doc/implementation/router-rust-migration-w-http-leaf.md`。

禁止：

- `run_router`/`main.rs`/`listener.rs` 生产装配（E-bootstrap gate 拥有）；
- `router/src/routing`、`router/src/dispatch`、router TS、
  `runtime/transport/src`、`runtime crate`、`deployment/`；
- AGENTS.md、scripts README、verify 注册表/selector graph/verify 文件、
  `scripts/skiff-instance.mjs`；
- 操作 stable instance / Mongo / PM2 / 4004-4007；不跑全量 `pnpm verify`。

## 自验收矩阵

| 验收项 | 命令 / 证据 |
| --- | --- |
| http 模块单测 + 真实 socket 探针 | `CARGO_TARGET_DIR=<worktree>/target cargo test -p skiff-router http` |
| 全部 router Rust 测试 | `node scripts/verify.mjs --only router-rust` |
| request.start 帧经 canonical codec byte-exact | 探针断言 fake dispatcher 收到 header + payload，且
  `encode_binary_frame` 后 decode 一致 |
| CORS/ceiling/backpressure 真实 socket | `http_*` 探针覆盖 preflight 204、显式 OPTIONS 透传、
  `ResponseTooLarge` 502、drain timeout → cancel |
| 写集干净 | `git status` 仅本 leaf + `router/src/http/` + lib.rs + `router/tests/http_*` |

## 交接

完成后向 `/root/router_rust_integration_b6` 报告 branch、worktree、提交
hash、测试命令与结果、surface-view seam 说明；同步通知 root。

## 执行结果（2026-08-02）

状态：完成。

- 交付文件：`router/src/http/`（mod/error/selector/ingress/cors/frame/
  stream/dispatch/fake/server）、`router/src/lib.rs`（additive）、
  `router/tests/http_common/` + `http_gateway_{unary,stream,selector_cors,
  disconnect_deadline}.rs`、本 leaf。
- 真实边界：`start_http_gateway`（hyper 1 + C-net 机制：Semaphore cap、
  watch + drain deadline + abort）→ `HttpDispatchPort` → fake dispatcher；
  未接 run_router（E-bootstrap gate 拥有生产装配）。
- 实现要点：
  - trusted selector：X-Skiff-Service / X-Skiff-Version（X-Skiff-Release
    alias + conflict 拒绝），singular/canonical token 校验；
  - service-scoped ingress：captured `Arc<RoutingEpoch>` +
    `HttpGatewaySurfaceView` → exact binding（deployment/gatewayEntryIdentity/
    mode/adapterKind）；OPTIONS 显式 binding → service-managed CORS；
  - typed `request.start`（canonical transport DTO + `encode_binary_frame`）
    + raw opaque body（request body limit → 413）；
  - unary HTTP phase 映射、stream response.start/chunk/end 串行 writer、
    chunk-before-start/end-before-start/end-with-payload fail closed、
    seq 由 fake dispatcher reference 状态机强制（C-model-request）；
  - cumulative response ceiling（unary payload 与 stream 累计 → 502
    `ResponseTooLarge`）；bounded channel + drain timeout → backpressure
    cancel；client disconnect → `client_disconnect` cancel（unary dispatch
    独立 task + oneshot，避免 handler 被 hyper drop 时销毁 pending
    correlation）；deadline → 504 `TimeoutError` + timeout cancel；
  - CORS 自动 preflight 204 / service-managed / 平台 error JSON；
    test-case correlation 头进入 `testEffectsEnabled`/
    `testCaseCapability`/`testCaseParentRequestId` 且从 httpRequest 剥离，
    自动 preflight 拒绝 correlation；
  - 早期错误路径先 drain request body 再响应，避免 TCP RST。
- 自验收：
  - `cargo test -p skiff-router http`：30 个真实 socket 探针全绿（多轮
    稳定）；
  - `cargo test -p skiff-router`：全量 router Rust 测试绿；
  - `node scripts/verify.mjs --only router-rust`：passed；
  - `cargo fmt --check -p skiff-router`：OK；新增文件 clippy 零告警。
- seam 记录：`RoutingEpoch` 当前投影不含 HTTP surface（mode/adapterKind），
  W-http 以 `HttpGatewaySurfaceView`（由 `DeploymentGatewayEntry` 构造）作为
  typed seam；E-bootstrap/E-http 接线时把 surface view 提升到 production
  composition（epoch 扩展或 supervisor 装配），不改变本模块 port 形状。
