# P5-F420B Retire RuntimeAssembly WebSocket residue

状态：Ready（F420A scope-expansion 后继）。

## 直接父节点

- `P5-F420A-i02-selector-and-terminal-generation-gate-result.md`

F420A 已闭合 terminal generation 与 I02 selector，完整 Router gate 的唯一 production 阻断是
遗留 `AssemblyWebSocketGateway` 仍把 WebSocket 当作 current RuntimeAssembly ingress。

该方向不再需要设计决策。现行权威模型已经冻结：

- `doc/architecture/gateway-runtime-adapter-boundary.md`：WebSocket 业务消息入口、selector/envelope 与
  identity 尚未冻结；旧字段应失败关闭或删除，不能作为目标设计依据。
- `P5-F362-router-runtime-assembly-v2-snapshot-result.md`：RuntimeAssembly v2 ingress 当前严格
  HTTP-only，WebSocket selector 是负例。
- `P5-F364-router-http-gateway-dispatch-result.md`：canonical RuntimeAssembly WebSocket dispatch
  已明确 fail closed，普通 connection lifecycle / `connection.send` 继续保留。

因此本节点删除 current production 对旧 Assembly WebSocket ingress 的接线与正例，不恢复
`contract`、`contractOperationId`、`websocketAdapter`、`websocketEntryId` 或旧 gateway identity
字段，也不设计新的 WebSocket 业务消息模型。

## 精确起点

- integrated start：
  `697246a15b9e1942b2e05be19d32d3c039c81786`；
- tree：
  `12734a8e1754b2898b55854df30c0e52559848da`；
- accepted F415：
  `7303af9bc9452a4d1d6e04e35b0eccb1ccacdc8d`。

启动时证明 start 与 F415 均为 HEAD ancestor且tree匹配。先实际列出 F420A 的 Router full
失败名与 TypeScript 当前错误，再修改。

## 独占范围

允许修改：

```text
router/src/gateway/assemblyWebSocketGateway.ts        # 允许删除
router/src/router/server.ts
router/src/index.ts
router/src/router/assemblyRuntimeRegistry.ts          # 仅删除已无调用者的 Assembly WS identity residue
router/tests/assembly-websocket-gateway.test.ts       # 允许删除
router/tests/assembly-websocket-ingress-identity.test.ts # 允许删除
router/tests/service-error-cross-layer-convergence.test.ts
router/tests/router-websocket-trust-dispatch.test.ts
router/tests/runtime-endpoint-connection-send-trust.test.ts
router/tests/actor-production-routing.test.ts
由 full Router listing 明确证明仍直接依赖上述已删除 Assembly WS surface 的其它 router/tests/**
本任务 result
```

禁止修改 artifact model/identity、compiler、deployment、runtime、scripts、test-runner、std、
ecosystem 仓库和锁文件。以下通用 WebSocket owner尤其必须保留，不得为了变绿而删除或改写语义：

```text
router/src/gateway/webSocketGateway.ts
router/src/gateway/webSocketConnectionLifecycle.ts
router/src/router/webSocketGenerationLifecycleRouter.ts
router/src/router/runtimeEndpoint.ts
router/src/router/runtimeDispatcher.ts
router/src/protocol/**
```

不得派子 Agent、merge/rebase/push、访问 stable/live、instance 或 watch registry。

## 必须实现

1. 删除 `AssemblyWebSocketGateway` production module、public export和`server.ts`启动/关闭接线。
   Router current production只启动已冻结的 Assembly HTTP ingress；HTTP server收到无owner的
   WebSocket upgrade时不能合成旧 Assembly binding。
2. 删除 `assemblyRuntimeRegistry.ts` 中仅服务旧 Assembly WebSocket ingress 的 identity DTO、
   canonical args/hash helper与未调用路径；HTTP/actor/service dispatch不变。
3. 删除只证明旧 Assembly WebSocket 正例的直接 test。不能把它们改写成带虚构字段的兼容 fixture。
   result 必须列出删除的 test 数与测试名，解释 full Router 总数的合法下降。
4. 混合测试只移除旧 Assembly WebSocket 半边；HTTP service-error convergence、普通
   `WebSocketGateway`、connection lifecycle、generation pin、runtime endpoint trust与
   `connection.send` 仍须实际执行。
5. `router-websocket-trust-dispatch` 与
   `runtime-endpoint-connection-send-trust` 如只因 snapshot fixture 仍含旧 WebSocket ingress而失败，
   改用 current HTTP-only/empty `gatewayIngress` snapshot，并继续测试普通 WebSocket wire/trust；
   不把它们改成 RuntimeAssembly WebSocket 正例。
6. `actor-production-routing.test.ts` 的固定 `2026-07-25` deadline 改为不会随日历过期的相对
   deadline；只修 test，Actor production语义不变。
7. 反搜 current production/test positive：
   - `AssemblyWebSocketGateway`、`canonicalAssemblyWebSocketIngressIdentity` 与
     `assemblyWebSocketGateway` import/export 为0；
   - RuntimeAssembly WebSocket selector/contract/adapter旧字段只允许出现在明确 rejection 负例；
   - 不得恢复 shared DTO字段、protocol alias、dual reader或fallback。

## 验证

先 listing再execution，所有既有 F420/F420A gate必须重跑：

```bash
pnpm --filter @skiff/router exec vitest list
pnpm --filter @skiff/router exec vitest run
pnpm --filter @skiff/router exec tsc --noEmit --pretty false

node --test \
  scripts/tests/artifact-identity-validation.test.mjs \
  scripts/tests/package-service-authoring.test.mjs \
  scripts/tests/package-service-i02-combined.test.mjs \
  scripts/tests/runtime-execution-boundary-checker.test.mjs \
  scripts/tests/skiff-source-test-suite.test.mjs

node scripts/check-artifact-identity-single-source.mjs --self-test
node scripts/check-artifact-identity-single-source.mjs

CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-test-runner \
  --test package_service_contract_deployment -- --list
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-test-runner \
  --test package_service_contract_deployment -- --test-threads=1

node scripts/run-skiff-tests.mjs
node scripts/verify.mjs --only router
node scripts/verify.mjs --only tooling

cargo fmt --all -- --check
git diff --check
```

至少额外证明下列保留面实际列出并通过：

```text
websocket-gateway.test.ts
websocket-connection-lifecycle.test.ts
websocket-generation-lifecycle-router.test.ts
router-websocket-trust-dispatch.test.ts
runtime-endpoint-connection-send-trust.test.ts
service-error-cross-layer-convergence.test.ts 的 HTTP 部分
actor-production-routing.test.ts
```

不得用删除 selector、跳过 test file、减少 verify plan或静态计数代替执行证据。

## 交付

实现与 `P5-F420B-retire-runtime-assembly-websocket-residue-result.md` 分开提交。result 记录：

- exact start/implementation commit/tree；
- 删除的 production/public surface 与测试清单；
- 保留的通用 WebSocket surface及实际计数；
- full Router 删除前后 test count、TypeScript 结果；
- F420/F420A 全部门禁实际结果；
- current positive与legacy negative反搜；
- F421 是否解除。

保持 worktree clean；不 merge/rebase/push。若闭合仍要求恢复共享 WebSocket artifact/protocol字段，
或发现新的范围外 production owner，立即 `TASK_SCOPE_EXPANDED` 停止并上报。
