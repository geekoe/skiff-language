# Router Rust Migration Batch 10 — WS-only routing Leaf Task

日期：2026-08-03
状态：execution leaf（一次性有界开发会话）
Agent：`/root/dev_ws_only_routing`
集成目标：`/root/router_rust_integration_b10`

## 引用链

- 批次文档：`doc/implementation/router-rust-migration-batch-10.md`
  （WS-only routing 节点：runtime/host control_plane 的 dispatch_modes 统计
  扩展为包含 WebSocket surface，加 WS-only deployment 路由测试）。
- 权威设计：`doc/implementation/router-rust-migration-plan.md`（draft v5），
  重点 §7 E-ws（capability enablement 不依赖 HTTP gate）与
  §8 `router-rust-ws-live`。
- 兄弟 gate leaf：
  - `router-rust-e-ws-gate-leaf.md`（Gap 4 后续明确记录残余缺口：真实
    Runtime `dispatch_modes` 只统计 HTTP gateway 条目，WS-only deployment
    广告空 modes → WS 真实链路 503；harness 以 HTTP raw unary 兜底条目
    绕行，残余交给 runtime owner 后续扩展）；
  - `router-rust-e-dispatch-gate-leaf.md`（root 裁决后的
    `dispatch_modes_from_gateway_entries` 派生实现与 E-dispatch harness）。
- 冻结契约：`router-rust-migration-c-routing-query-contract.md` §3 规则 5
  （`mode == unary` 要求 `capabilities.unary`；WS connect selector 以
  `DispatchMode::Unary` 查询候选）。

## 基线

- 集成基线：`origin/main@edc111f8`（Batch 9 已 push；E-ws/E-dispatch gate
  已合入）。
- 共享主 worktree 只读；本节点 worktree：
  `/Users/geek/workspace/wt-ws-only-routing`，分支
  `feat/router-rust-ws-only-routing`。

## 零 worktree 只读预检结论（锚定 edc111f8）

1. `runtime/host/src/host/control_plane.rs::queue_runtime_capabilities`
   从已 admit assembly 的 `candidate().gateway_entries()` 派生
   `dispatch_modes`；`dispatch_modes_from_gateway_entries` 只匹配
   `GatewayProtocolSurface::Http`，WebSocketConnect / WebSocketJsonRpc
   条目不贡献任何 capability。
2. `router/src/supervisor/ws.rs::ProductionWsConnectSelector` 以
   `CandidateQuery { mode: DispatchMode::Unary }` 经
   `RuntimeCandidateQuery` 选候选；C-routing-query 规则 5 把无 unary
   capability 的 session 排除 → WS-only deployment 无候选。
3. `GatewayWebSocketConnectProtocolSurface` 无 dispatch_mode 字段（connect
   handler 是一次性 unary 握手）；`GatewayWebSocketJsonRpcProtocolSurface`
   有 `dispatch_mode` 字段，compiler 当前恒投影
   `GatewayDispatchMode::Unary`（`compiler/driver/websocket_gateway_projection.rs`）。
4. E-ws harness `scripts/check-router-ws-live.mjs` 的 artifact authoring
   含一个 HTTP raw unary 兜底条目（`http.yml` ping + `main.ping` handler），
   使 runtime 诚实广告 unary；probe `router/tests/ws_live_probe.rs` 不引用
   ping/HTTP 条目。
5. 本节点不改 Router 生产代码、不改 harness 的 probe 与 registry/CI。

## 任务目标

把 runtime dispatch_modes 统计从仅 HTTP surface 扩展为包含 WebSocket
surface，使 WS-only deployment 也能被 `RuntimeCandidateQuery` 选中：

- WebSocketConnect 表面贡献 unary（connect 握手是 unary dispatch）；
- WebSocketJsonRpc 表面按其 `dispatch_mode` 贡献 unary/serverStream；
- HTTP surface 行为保持不变（unary/serverStream 按 `dispatch_mode`）。
- 测试覆盖 HTTP-only、WS-only、混合三种 deployment 的 capability 广告。
- E-ws harness 去掉 HTTP 兜底条目后仍 PASS（真实 WS-only artifact 经真实
  Router + Runtime 全链路由成功）。

## 实现决策

### 1. runtime capabilities 扩展（`runtime/host/src/host/control_plane.rs`）

`dispatch_modes_from_gateway_entries` 的 match 扩展为：

- `GatewayProtocolSurface::Http(http)`：按 `http.dispatch_mode` 置位
  unary/serverStream（不变）；
- `GatewayProtocolSurface::WebSocketConnect(_)`：置位 unary（connect 握手
  是 unary 表面；该 surface 没有 dispatch_mode 字段）；
- `GatewayProtocolSurface::WebSocketJsonRpc(rpc)`：按 `rpc.dispatch_mode`
  置位 unary/serverStream。

返回顺序保持固定的 `[unary, serverStream]`。`queue_runtime_capabilities`
的注释同步更新为 HTTP + WebSocket surface 共同投影。

同文件 `#[cfg(test)] mod tests`：

- 保留：无 surface → `[]`；HTTP unary → `[unary]`；HTTP serverStream →
  `[serverStream]`；HTTP both → 固定顺序；fresh host（无 admit）capability
  帧为空。
- 更新：websocket connect-only surface → `[unary]`（原断言为 `[]`，编码了
  残余缺口）。
- 新增：websocketJsonRpc unary → `[unary]`；websocketJsonRpc serverStream
  → `[serverStream]`（类型允许，防御性覆盖）；三种 deployment 级广告：
  HTTP-only（unary + serverStream）、WS-only（connect + jsonRpc →
  `[unary]`）、混合（HTTP unary/serverStream + WS connect/jsonRpc →
  `[unary, serverStream]`）。

### 2. E-ws harness 去 HTTP 兜底（`scripts/check-router-ws-live.mjs`）

- 删除 `http.yml` authoring 块与说明注释；
- 删除 `main.skiff` 中的 `ping` handler（服务变为纯 WS gateway）；
- `router/tests/ws_live_probe.rs` 不需要改动（不引用 ping/HTTP）。

该文件是本任务验证方式点名要改的 E-ws harness（非 verify 基础设施：
`scripts/verify.mjs` / `scripts/lib/verify-*` / `scripts/tests/verify-*`
均不触碰）。

### 3. 文档

- 本叶子任务文件（此文件）；
- `router-rust-e-ws-gate-leaf.md` 的 Gap 4 后续小节追加 Batch 10 处理
  记录（残余缺口已由本节点关闭，含实现位置与验证方式）。

## 写入边界

可写：

- `runtime/host/src/host/control_plane.rs`（唯一 runtime 生产文件）；
- `runtime/host` 相关测试（同文件 tests module）；
- `scripts/check-router-ws-live.mjs`（仅删 HTTP 兜底条目 + 相关注释）；
- `doc/implementation/router-rust-ws-only-routing-leaf.md`（本文件）；
- `doc/implementation/router-rust-e-ws-gate-leaf.md`（仅追加本节点处理
  记录）。

禁止：

- `router/src`、`runtime/transport/src`、`deployment/`、router TS、
  AGENTS.md、scripts README、verify 基础设施文件（`scripts/verify.mjs`、
  `scripts/lib/verify-*`、`scripts/tests/verify-*`）、`skiff-instance.mjs`；
- 操作 stable instance / Mongo / PM2 / 4004-4007；不跑全量 verify。

## 自验收矩阵

| 项 | 命令 / 证据 |
| --- | --- |
| capability 广告测试（HTTP-only/WS-only/混合） | `cargo test -p skiff-runtime-host control_plane` 全绿 |
| 全 runtime-host crate 回归 | `cargo test -p skiff-runtime-host --no-fail-fast` |
| WS-only 真实链路 | `node scripts/check-router-ws-live.mjs`（无 HTTP 兜底条目）PASS |
| 格式 | `cargo fmt -p skiff-runtime-host -- --check` |
| 写集干净 | `git status` 仅本叶子声明文件 |

## 交接

完成后提交到 `feat/router-rust-ws-only-routing`（不 push），直接向
`/root/router_rust_integration_b10` 报告 branch、worktree、commit/tree、
实际写集、自验收矩阵与已知残留（如有），并通知 root。
