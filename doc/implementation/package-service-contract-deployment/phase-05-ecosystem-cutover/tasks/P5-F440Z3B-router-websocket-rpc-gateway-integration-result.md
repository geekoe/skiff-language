# P5-F440Z3B Router WebSocket RPC Gateway integration result

状态：`TASK_SCOPE_EXPANDED / CURRENT_WEBSOCKET_CONNECT_IDENTITY_OWNER_REQUIRED`。

本 leaf 已按 test-first 要求命中真实 Gateway production 缺口，但在把 current Z1 physical
binding 接到真实 connect dispatch 时，证明必须修改本任务禁止的 wire owner 与
cross-system fixture。按任务停止条款，没有把 v1/v2 compatibility、dual read或临时 identity
转换塞进 Gateway；所有未完成 production/test WIP 已清理，未创建 implementation commit。

## 1. 基线与 worktree

| 项目 | 值 |
| --- | --- |
| 任务声明 integration baseline | `78c10985` |
| worktree 实际起点 | `1114df2d848acca8d0519d52303061ebdad8fb5a` |
| worktree | `/Users/geek/workspace/skiff-p5-f440z3b-router-rpc-gateway` |
| branch | `codex/p5-f440z3b-router-rpc-gateway` |
| implementation commit | 无；scope blocker 前的 WIP 不满足安全提交条件，已清理 |
| result commit | 本文独立提交；最终 commit/tree 由交付消息记录 |

`78c10985..1114df2d` 只包含 Z3A result 与本 leaf 调度文档，没有本 leaf production
实现。

## 2. 真实 test-first RED

先在真实 loopback `AssemblyWebSocketGateway` fixture 中给 handlerless physical binding
加入一个 captured `websocketMethods` entry，并要求 upgrade 执行一次 connect/acquire：

```text
router/node_modules/.bin/vitest run --root router \
  tests/websocket-gateway.test.ts \
  -t 'eagerly pins a handlerless method-bearing WebSocket connection'
```

实际执行 `1` 个测试，`1 failed`：

```text
AssertionError: expected [] to have a length of 1 but got 0
tests/websocket-gateway.test.ts:64
```

这证明 current Gateway 的 `binding.handler === undefined` 分支真实跳过了 method-bearing
connection 的 connect dispatch；不是零 selector、synthetic throw或 dependency failure。
在 scoped WIP 中把判定改为 `handler exists OR captured method table non-empty` 后，同一
targeted test `1/1 PASS`，且当时 `pnpm --dir router type-check` PASS。

该 WIP 随后进入 current v2 physical binding 探针时触发下面的禁止-owner blocker，因此没有
作为 implementation 提交保留。

## 3. TASK_SCOPE_EXPANDED 证据

Z1 current snapshot reader只接受并向 Gateway 暴露
`skiff-gateway-entry-v2:sha256:<digest>`：

- `router/src/router/runtimeAssemblyWebSocketSnapshot.ts:12` 的 physical/method decoder pattern
  为 v2；
- `webSocketGateway.ts:636` 与 `:659` 把同一个 selected
  `binding.gatewayEntryIdentity` 原样放入 connect routing/metadata；
- `webSocketGateway.ts:663` 随即调用 TypeScript connect wire validator。

但 current TypeScript connect wire仍只接受 v1：

- `router/src/protocol/runtimeAssemblyRequest.ts:296-301` 只有
  `websocketJsonRpc` 使用 v2，`websocketConnect` 与 HTTP 一起落入 legacy v1；
- `router/src/protocol/runtimeAssemblyRequestMetadata.ts:147-152` 对
  `websocketConnect.gatewayEntryIdentity` 强制 legacy v1；
- `router/src/protocol/runtimeProtocol.ts:1612-1615` 与 `:1691-1694` 的 connect schema
  两处均为 v1。

与此同时，Rust canonical owner已经只接受 v2：

- `artifact-model/src/compile_identity.rs:66` 的
  `GATEWAY_ENTRY_IDENTITY_PREFIX` 为 `skiff-gateway-entry-v2:sha256`；
- 同文件 `:212-227` 的 `GatewayEntryIdentity::parse` 只按该 prefix解析；
- `runtime/transport/src/runtime_assembly_request/lexical.rs:180-187` 的 connect/request
  deserializer直接调用该 parser。

因此不能在 Gateway 中选择一个既通过 TypeScript connect validator、又能被 current Rust
runtime接受、还与 selected current physical binding identity相等的值。

真实 current v2 Gateway probe 使用 loopback HTTP/WebSocket server，直接得到：

```text
Error: Unexpected server response: 500
```

失败发生在 upgrade 内的 `assemblyWebSocketConnectRequestHeader` validation，尚未到 bridge
attach。该 run 已先用 `vitest list` 精确列出 `10` 个非零 Gateway integration tests；执行时
所有依赖 method-bearing connect 的以下场景都被同一 HTTP 500 遮挡：

- handlerless eager pin + snapshot replacement/old receipt；
- inbound success/notification/cancel；
- source/replica/service/entry fail-closed；
- pinned runtime disconnect `1011`；
- attach/upgrade failure accounting；
- shutdown teardown/release accounting。

Pure path-only场景不走 connect dispatch，因此没有被 identity blocker遮挡；草稿中一个
writer/disconnect组合已通过，其余尚有 terminal/timing断言未收敛。触发停止条款后未继续调试，
也未保留这些不完整测试。

## 4. 必须新增的 owner

要解除本 leaf，必须先有一个明确 checkpoint 同时拥有：

1. TypeScript `websocketConnect` request validator、metadata validator与generated/runtime schema
   的 v1 → current v2 hard cut；
2. `runtime-assembly-request-wire.test.ts` 与 connect protocol/dispatch direct fixtures的同步刷新；
3. 当前仍冻结 v1 的
   `cross-system-fixtures/package-service-ecosystem/runtime-websocket-connect-wire.json` 更新。

第 3 项明确属于后继 F0 且被本 leaf 禁止修改；前两项的 production protocol files也不在本
leaf 写入范围。把 v1、v2同时接受，或在 Gateway 中把 prefix改写为 v1，都会违反
“不改变 wire语义 / 不新增 compatibility adapter / 禁止 dual read”的任务约束。

建议先落一个独立 current websocketConnect wire identity checkpoint，再从该 checkpoint重新调度
Z3B；随后 F0只消费已经一致的 canonical wire更新 cross-system corpus。

## 5. 提交、验证与范围审计

- 没有修改 wire、RuntimeDispatcher、RuntimeEndpoint、broker、Host/Rust或
  cross-system fixture；
- 没有保留 Gateway/server/测试 WIP，避免提交一个 current production必然 HTTP 500 的半成品；
- 没有启动 stable instance、watch、长期 server、外部 network或 live selector；
- loopback server与临时 dependency symlink均已回收；
- 未派子 Agent，未 merge、rebase或 push；
- 任务规定的最终九文件 GREEN、最终 type-check不能成立，原因是上述禁止-owner blocker，而不是
  零测试或环境缺依赖。

Result提交前的 worktree除本文外为 clean；`git diff --check` 由提交前验证记录。
