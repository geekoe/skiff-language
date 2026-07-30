# P5-F341 Service error Router consumer and external mapper

状态：Completed。结果见
`P5-F341-service-error-router-consumer-result.md`。

## 直接父节点

- 当前 Router 跳点、R owner、W1 probes与外部安全策略：
  `P5-F333-wire-observability-delta-audit-result.md`
- 已冻结并通过复验的 shared response.error v2 checkpoint：
  `P5-F339-response-error-schema-reacceptance-result.md`

父节点已沿引用链连接唯一权威设计。本任务只实现 R：Router endpoint/dispatcher/gateway consumer；
不改 shared protocol/schema/corpus、Rust或 telemetry service。

## 起点与目标

- 起点 commit：`e3095ec642d49b59955f5f48a2950eafc9d92571`
- 起点 tree：`6b7fce6db07d7fde3b88609539150c53f5608e62`
- Router只按 C0 `errorKind`和 strict service envelope view分流，不按
  code/message/status/details推断 fixed。
- runtime endpoint必须用
  `validateResponseErrorFrame(header, payloadBytes)`接收完整 v2 frame：
  - fixed header无 generic error，保留同一个 payload `Uint8Array`；
  - control header有 generic error，payload为空；
  - malformed/mixed/legacy v1 fail closed。
- `unaryFrame`是 service-to-service转发路径：fixed/control都直接返回收到的 exact header与
  exact payload对象/bytes，不重建 header、不 stringify/re-encode service envelope。
- 普通 unary/stream pending显式分流：
  - control仍进入`RuntimeResponseError`与既有 generic status classifier；
  - fixed进入新的 fixed-specific安全错误类型/mapper，绝不进入
    `RuntimeResponseError`或`runtimeErrorStatus`。
- 当前没有公开 typed HTTP error adapter。所有 public/Internal/platform fixed对外均 fail closed为
  脱敏 5xx，只暴露稳定通用信息与 safe traceId/errorId；不暴露 provider message、payload、source、
  path、function、frames或stack。
- HTTP与WebSocket upgrade/close reason使用同一个 fixed安全事实；WebSocket reason满足123-byte限制。
- runtime-originated不支持的`request.start`拒绝仍是 generic control，但必须写 C0 v2 control header与
  空 payload。

## Production 写入边界

唯一允许 production 写入：

- `router/src/router/{runtimeEndpoint.ts,runtimeDispatcher.ts,errors.ts,assemblyHttpGateway.ts}`；
- `router/src/gateway/assemblyWebSocketGateway.ts`。

明确禁止修改：

- `router/src/protocol/**`；
- `router/src/router/httpGateway.ts`（由 T 仅处理 telemetry literal）；
- `router/src/telemetry/**`；
- shared corpus；
- Rust、telemetry service、权威设计、父 task/result、package/lockfile。

不得修改旧非 assembly `router/src/gateway/webSocketGateway.ts`来代替 production
`AssemblyWebSocketGateway`。

## 必须实现并证明

1. endpoint v2 control producer与 fixed/control strict admission正确；所有 payload presence规则由同一
   C0 seam执行。
2. dispatcher `unaryFrame` fixed exact bytes/header转发；control也保持 v2，不回写 v1。
3.普通 pending fixed/control使用互斥错误类；matching code/message的 control不能升级 fixed。
4. fixed error类只保存或公开 finite kind/correlation；若内部保留 raw bytes，也不得进入
   `message/details/toPayload/toHttpBody/WebSocket reason`。
5. Assembly HTTP fixed响应为稳定脱敏 5xx并带 traceId/errorId；现有 generic GatewayError输出同时使用
   既有 5xx detail redaction policy，不再从`toPayload()`泄漏5xx details。
6. Assembly WebSocket connect/receive fixed失败的 upgrade body或close reason包含至多稳定通用信息与
   correlation；不得出现 private sentinel、callee source/path/function/stack。
7.旧 v1 response.error fixtures/producers在本 owner范围内删除或改为 v2；没有 dual read/write/fallback。

## 测试与验证

允许测试写入：

- `router/tests/helpers/runtime.ts`；
- `router/tests/{runtime-protocol-websocket-response.test.ts,
  runtime-assembly-unary-dispatch.test.ts,assembly-runtime-endpoint.test.ts,
  runtime-registry-dispatch.test.ts,assembly-http-gateway-stream.test.ts,
  assembly-websocket-gateway.test.ts,runtime-errors.test.ts}`。

只修改直接相关 focused cases；不得触碰`router/tests/http-telemetry.test.ts`（T owner）或 shared protocol
test/corpus。

至少覆盖：

- fixed public/Internal/platform unaryFrame exact header/payload reference或byte equality；
- control v2空 payload、matching Internal code/message仍 generic；
- legacy v1、mixed fields、fixed空/control非空payload失败；
- ordinary fixed绕过 RuntimeResponseError/status classifier；
- Assembly HTTP/WS外部不泄露反搜和 trace/errorId；
- runtime-originated request.start的 v2 control拒绝。

先列出非零 selector，再运行上述文件的 focused Vitest集合，以及：

```bash
pnpm --filter @skiff/router run type-check
git diff --check
```

并行 T 尚未合流时，type-check只允许报告 T 独占的
`router/src/router/httpGateway.ts`/`router/tests/http-telemetry.test.ts` visibility断点；本任务及其它
文件必须无报错。不得运行完整 workspace/root、stable/live，不 push。

## 交付

- worktree：`/Users/geek/workspace/skiff-p5-f341-service-error-router`
- branch：`codex/p5-f341-service-error-router`
- 新的一次性开发 Agent；
- 新增`P5-F341-service-error-router-consumer-result.md`，列出 wire转发、固定错误外部映射、泄露反搜、
  selector/数量与剩余 blocker；
- 提交并返回 implementation commit，不修改 task 状态，不承接后续验收。
