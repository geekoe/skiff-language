# Batch 10 — E-actor-parity Differential Scenarios（actor_parity_*）

日期：2026-08-03

> 已由 M4 取代（2026-08-06，runtime-lazy-deploy）：本文所述 activation 协调层 / epoch 部署机制
> （`assembly.activation` 帧族、`/__skiff/activate-assembly`、activation 状态仓库与配置项、
> committed/expected generation 术语等）已在 M4 全部下线，部署语义以
> [`doc/architecture/runtime-lazy-load-deployment.md`](../../../architecture/runtime-lazy-load-deployment.md)
> 为准。本文保留为历史执行记录。
归属：E-actor-parity 节点（`router-live:actor` 扩展）；共享 W-differential
inventory 文档由 differential 扩展节点统一维护，本文件只记录
`actor_parity_*` 前缀场景。

## 场景清单

| id | status | lane | 说明 |
| --- | --- | --- | --- |
| `actor_parity_full_chain` | runnable | actor | two-replica actor
get-or-create/invoke/owner-control/lease/function-spawn/actor-method-spawn
full chain：TS/Rust Router 各自消费同一 canonical actor-routing projection，
相同 real-HTTP 驱动、两个 real Runtime replica（test-only relay）、独立
port/artifact root/runtime home/Mongo namespace；HTTP steps、投影帧序列、
Mongo state/audit、terminal 无未解释差异 |

Inventory：`scripts/fixtures/router-differential/actor_parity_inventory.json`
（schemaVersion 与共享 inventory 一致，baseline
`edc111f888a70743a8ecadc3bdbcb6b4ae2fd54a`）。

Projection 输入：两侧 Router 消费同一**真实**（非空）canonical
actor-routing projection，由 harness 以 test-side A1 producer 角色从
artifact 的 PackageArtifact/File IR 记录合成（
`scripts/lib/router-differential/actor_parity_projection.mjs`，canonical
JSON 与 skiff-canonical-json 一致）。空 projection 在本 gate 不可用：TS
A2 硬切对 `actor.method.invoke` 的 UnknownMethod fail-closed 与 Rust
旧转发行为构成未解释差异，本批已在 Rust sink 侧补 catalog admission
（miss → 不转发、不写 error frame，与 TS 语义一致），并使 probe 消费真实
记录而非覆写为空。

## 观察类型与对比契约

- `http.steps`（equal）：每步 `{name, status, body}`；成功步 body 为解析后的
  JSON 值；失败步 body 为归一化 platform error
  `{code, message, details}`（details 内 traceId/errorId/UUID 等 opaque 值
  替换为 `<opaque>`）。
- `frameEvents.<replica>`（equal）：投影后的语义帧序列，逐 replica
  保持记录顺序；每帧 `{direction, type, replica, payloadSha256, fields}`。
- `http.controlHealthStatus`（equal）、`mongo.state`/`mongo.auditCount`
  （equal）、`terminal.*`（equal）。
- recordOnly：`http.publicStatus`、`http.controlHealthBody`（Rust health
  JSON 仍为空占位，不参与 equal）、`timings`（驱动步耗时证据）、
  `rawFrames`（relay 原始帧，含握手/health）、`logs`。

## 帧投影政策（actor_parity 专用；不属于共享 normalization kinds）

1. 排除帧：`router.bootstrap`、`runtime.capabilities`、
   `assembly.activation`、`runtime.registered`、`runtime.health`（握手与
   周期 health 帧的语义由 session-handshake 场景与其他 gate 覆盖；
   health 帧无语义顺序）。
2. ephemeral 关联 id 按 **key** 独立替换为 `<key-N>`（首次出现顺序）：
   `rpcId`、`requestId`、`invocationId`、`spawnId`、`claimId`、
   `ownerLeaseId`、`evictionRequestId`、`traceId`、`spanId`、
   `parentSpanId`、`errorId`、`activationId`。同 key 的跨帧关联在两侧各自
   保留；格式差异（TS `actor-owner-<uuid>` vs Rust `owner-lease-<n>`）不
   视为语义差异。
3. timestamp 字段（`expiresAt`、`observedAt`）→ `<timestamp>`。
4. `request.start` 只保留语义路由字段（mode/caller.kind/routing 身份与
   gateway entry/ingress/deadline timeoutMs/requestId/httpRequest
   method+path）；丢弃含本侧端口的 url、trace 细节。
5. 非空 payload 计算 `payloadSha256`（真实 Runtime 同 binary，业务 payload
   确定性一致）。
6. 其余字段（actor key、abi/impl identity、declarationOwner、epoch、
   operation、accepted、error code 等）原样保留并参与 equal。

未声明共享 normalization kinds：投影在 capture 阶段完成，inventory
`normalizations: []`；`rawFrames` 保留原始证据供人工审计。

## 与既有 gate 的关系

- `scripts/check-router-actor-live.mjs` = Phase 1（Rust-only
  `actor_live_probe` 负例/竞态/归零回归层）+ Phase 2（本 differential，
  parity 证据）。
- checked-in task expectation（`verify-live-registry.mjs` 的
  `router-live:actor` description）已更新为 TS/Rust differential，
  requiredExecutables 增加 `pnpm`，requiredModules 增加
  `ws`（router/package.json），与 Phase 2 的实际前置一致。
- CI `Router Rust Actor (managed)` job 安装 pnpm + TS router deps 后运行
  同一脚本；change classifier 覆盖 `actor_parity_*` 路径。

## 已记录并接受的差异（root 裁决 2026-08-03）

differential 全链按 **accepted-with-recorded-differences** 交付：
`http.steps`（含 flaky 失败步）、health、Mongo、terminal 全部 equal；
`frameEvents` 对比的 2 项失败（replica-1/replica-2）全部落在 inventory
`knownDifferences` 声明的已接受路径上（`accepted: true`），不再阻塞 gate。
runner 只允许失败路径精确命中已声明 knownDifferences；出现任何未声明路径
仍 fail closed。

1. **flaky/retained-entry 失败路径**（accepted；id
   `flaky-retained-entry-failure-stage`）：TS 对 retained entry 的第二次
   getOrCreate 直接 resolve 并在 method invoke 时延迟激活（失败经
   `actor.owner.failure` 显现）；Rust 在第二次 getOrCreate 重新 activation
   （ACK rejected → `actor.getOrCreate.error`）。两侧均 fail closed，HTTP
   可观测（status/body）完全一致。Owner 建议：TS
   `ActorGetCreateActivationCoordinator`/`ActorMethodDispatcher` vs Rust
   `ActorActivationRequestBroker`/`ActorFrameSink`；cutover 后 Rust 为唯一
   实现，不另派语义修复；未来如需 TS 对齐再单独立项。
2. **rejected activation 的 `actor.getOrCreate.error` code**（accepted；id
   `rejected-activation-error-vocabulary`）：TS `ActorCreateFailed` vs Rust
   `AckRejected`（Rust activation corpus 冻结词汇）。Owner 建议：TS
   coordinator 错误映射 vs Rust broker waiter outcome 词汇；同上不另派修复。
3. **异步帧交织顺序**（non-blocking follow-up；id
   `async-frame-interleaving-order`）：成功路径两侧帧集合一致，独立子流
   （spawn 提交/返回、owner.invoke/control）到达顺序非语义，当前按 relay
   记录序比较产生顺序 false positive。后续可按 HTTP 步窗口 canonical
   排序收敛；本批未实施，避免掩盖语义差异。
