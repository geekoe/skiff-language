# P5-F364 Router HTTP gateway dispatch

状态：Ready（C3 Router HTTP request/response leaf；与 Runtime request/eval seam 并行）。

## 直接父节点

- `P5-H36-external-ingress-implementation-dag.md`
- `P5-F359-http-gateway-request-protocol-result.md`
- `P5-F360-typed-json-unary-correction-result.md`
- `P5-F362-router-runtime-assembly-v2-snapshot-result.md`

父节点已冻结 HTTP-only canonical request、RuntimeAssembly v2 exact snapshot、typedJson unary、
rawHttp unary/server-stream及timeout owner。本任务只把外部HTTP socket请求投递到该canonical
request，并把runtime HTTP response写回客户端；不设计WebSocket业务消息。

## Exact base

- integration commit：`b71e622ca35109519e904f269a67f19bc2f08de4`
- integration tree：`7d79c140534db0ed2336e3babe511fa444fdc2e6`
- branch：`codex/package-service-phase-05`

当前 `assemblyHttpGateway.ts` 仍生成 `caller.target/contractOperationId/testEffectDoubles`；
`assemblyRuntimeRegistry.ts`仍按ServiceContract operation验证HTTP request；
`runtimeDispatcher.ts`仍把canonical RuntimeAssembly request解释成WebSocket receipt/phase。它们与
F359/F362当前shape不一致。

## 必须完成

1. `assemblyHttpRequestHeader`只构造F359 canonical header：
   - `caller`只有`{kind:"gateway"}`；
   - routing只含assembly identity/generation、nested `gatewayEntryIdentity`与HTTP ingress；
   - mode来自exact snapshot binding；
   - `httpRequest`携带平台metadata，binary payload是原始body bytes；
   - 无contract operation、handler/adapter/schema、top-level gateway identity、test double或
     WebSocket字段。
2. HTTP selector lookup必须精确命中当前 committed snapshot binding。Registry逐值验证：
   - assembly identity/generation；
   - canonical selector；
   - nested gateway entry identity；
   - request mode；
   - HTTP request URL/host/method/path与routing一致。
   不加载ServiceContract operation，不解析adapter plan或external schema。
3. Router effective timeout 为：

   ```text
   min(platform HTTP cap, deployment policy.timeoutMs when present)
   ```

   `requestTimeoutMs`及其默认值是platform cap；同一effective值同时用于request deadline与dispatcher
   timer。非法/零/unsafe值fail closed，不静默回退到更长timeout。
4. unary：
   - rawHttp status/headers/body逐值写回；
   - typedJson body仍是opaque bytes，Router不解码/重编码；
   - response byte ceiling、client disconnect、timeout与single terminal保持。
5. server stream只可能来自rawHttp binding：
   - runtime必须恰好先发一次response.start(status/headers)，再发零或多个binary chunk，最后end；
   - start/chunk/end顺序、payload/metadata约束fail closed；
   - HTTP backpressure、drain timeout、client disconnect与runtime cancel继续闭合；
   - Router不得把typed JSON、SSE event或业务chunk结构加入wire。
6. `assemblyRuntimeRegistry.ts`和`runtimeDispatcher.ts`中把canonical request当作旧
   RuntimeAssembly WebSocket request的分支必须删除或明确fail closed。保留普通legacy runtime
   dispatch及通用connection lifecycle，但不得创建本地WebSocket兼容DTO或把旧字段塞回F359 header。
7. 更新HTTP与registry/dispatcher直接tests，至少覆盖：
   - raw unary、typed unary、raw server-stream；
   - exact nested gateway identity、wrong generation/selector/identity/mode；
   - body非UTF-8仍opaque；
   - deployment timeout小于platform cap、无override、platform cap更小；
   - start/chunk/end、backpressure、disconnect、timeout与oversize负例。

## 写入范围

主要owner：

- `router/src/router/assemblyHttpGateway.ts`；
- `router/src/router/assemblyRuntimeRegistry.ts`；
- `router/src/router/runtimeDispatcher.ts`；
- `router/src/router/httpStreamResponseWriter.ts`（仅直接需要时）；
- 上述owner的直接tests/fixtures。

禁止：

- `runtimeAssemblySnapshot.ts`、`runtimeAssemblyDeploymentSnapshot.ts`及filesystem loader；
- `router/src/protocol/**` canonical wire owner；
- `router/src/gateway/assemblyWebSocketGateway.ts`及WebSocket业务/connection authoring；
- `assemblyControlPlane.ts`、test-runner、Rust Host/runtime、compiler、三仓库service；
- stable/live配置、lockfile。

完整Router type-check允许只剩禁止范围内的明确旧WebSocket/control-plane/fixture consumer错误；
本任务owned production与direct tests不得有错误。若HTTP正确实现要求修改F359/F362公共DTO，立即返回
`TASK_SCOPE_EXPANDED`。

## 验证

先枚举实际非零test files，再运行：

```bash
pnpm --filter @skiff/router exec vitest run \
  tests/assembly-http-gateway-stream.test.ts \
  tests/assembly-replica-dispatch.test.ts \
  tests/runtime-assembly-unary-dispatch.test.ts
pnpm --filter @skiff/router exec tsc --noEmit --pretty false
git diff --check
```

记录完整type-check所有剩余错误并证明owned production/tests为零。反向搜索owned production路径不得
剩余`contractOperationId|callerTarget|testEffectDoubles|websocketAdapter|websocketEntryId`。
不运行stable/live、root完整gate，不push。

## Worktree与交付

- worktree：`/Users/geek/workspace/skiff-p5-f364-router-http-dispatch`
- branch：`codex/p5-f364-router-http-dispatch`
- 从包含本task的integration checkpoint创建；
- production/tests一个commit，result一个commit；
- result写入`P5-F364-router-http-gateway-dispatch-result.md`；
- worktree保持clean，不merge/rebase integration，不push。
