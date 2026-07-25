# P5-F341 Service error Router consumer and external mapper result

状态：实现完成；F341 自身无 blocking。未修改 task 状态，未 push，未承接后续节点。

## 候选与边界

- worktree：`/Users/geek/workspace/skiff-p5-f341-service-error-router`
- branch：`codex/p5-f341-service-error-router`
- 任务声明起点 commit：
  `e3095ec642d49b59955f5f48a2950eafc9d92571`
- 任务声明起点 tree：
  `6b7fce6db07d7fde3b88609539150c53f5608e62`
- worktree 启动 HEAD：
  `5fa0389e151a27f9cbd2906a7394658190e42491`
  （只比声明起点多 fan-out task 文档）
- implementation/result 由本交付 commit 同一提交承载；最终 commit 以交付消息为准。

production 写入严格限制为：

- `router/src/router/{runtimeEndpoint.ts,runtimeDispatcher.ts,errors.ts,assemblyHttpGateway.ts}`
- `router/src/gateway/assemblyWebSocketGateway.ts`

测试写入严格限制为叶子任务允许的 focused 文件及
`router/tests/helpers/runtime.ts`。没有修改 Router shared protocol、shared corpus、Rust、
telemetry、`router/src/router/httpGateway.ts`、package/lockfile或父 task/result。

## Wire admission 与 exact 转发

1. `RuntimeEndpoint`在任何 generic header dispatch 前，对所有
   `response.error`调用唯一 C0 seam
   `validateResponseErrorFrame(frame.header, frame.payloadBytes)`：
   fixed 非空 strict envelope、control 空 payload、mixed/legacy/malformed和错误 payload presence
   均由该 seam fail closed。
2. runtime-originated、不支持的 service `request.start`拒绝现在写
   `skiff-runtime-frame-v2`、`errorKind: control`、generic error和空 payload；没有 v1 producer。
3. `RuntimeDispatcher.rejectRequest`只接收 C0 的
   `ValidatedResponseErrorFrame`。`unaryFrame`对 fixed/control都直接返回 seam 给出的同一个
   `header`和`payloadBytes`对象，不重建 header、不复制或重编码 payload。
4. 普通 unary/stream pending按 typed union互斥分流：
   fixed只创建`FixedServiceResponseError`；control才创建`RuntimeResponseError`并进入既有
   generic status classifier。matching `InternalError` code/message的 control仍为 generic。

## Fixed 外部安全映射

- `FixedServiceResponseError`只从 strict view保留 finite
  `publicTypedError | internalError | platformError` kind及`traceId/errorId`；不保存 raw envelope、
  encoded payload、provider message、source、path、function、frames或stack。
- 当前没有公开 typed HTTP adapter，因此三种 fixed kind统一 fail closed为：
  HTTP 500、稳定 code `FixedServiceError`、稳定 message `Service request failed`和
  `{traceId,errorId}` correlation。
- `AssemblyHttpGateway`改用基于`toHttpBody()` policy的`toHttpPayload()`。
  fixed override只放行 correlation；所有其它 generic `GatewayError`/`RuntimeResponseError`
  5xx details按既有 policy删除，不再调用会原样暴露 details 的`toPayload()`。
- `AssemblyWebSocketGateway`的 upgrade body与receive close reason都从同一
  `FixedServiceResponseError.toExternalMessage()`取得稳定 message和 correlation；不读取 fixed
  payload或provider message。close reason按 Unicode code point截断并严格保持最多123 UTF-8 bytes。

## Leakage 反搜

production允许写入面反搜以下 sentinel/diagnostic字段为零：

```text
provider-private-secret
/callee/private/source.skiff
calleePrivateFunction
sourceFrames
stack（word match）
module_path
symbol_path
```

测试把相同 sentinel放入 Internal message以及 public/platform encoded payload，随后反搜 HTTP bytes、
WebSocket upgrade bytes与close reason；均只看到稳定通用信息和 correlation。fixed class的
`details`保持`undefined`，HTTP/WS serializer没有 raw bytes入口。

## Selector 与验证

先执行：

```bash
pnpm --dir router exec vitest list \
  tests/runtime-errors.test.ts \
  tests/runtime-assembly-unary-dispatch.test.ts \
  tests/assembly-http-gateway-stream.test.ts \
  tests/assembly-websocket-gateway.test.ts \
  tests/assembly-runtime-endpoint.test.ts \
  tests/runtime-registry-dispatch.test.ts \
  tests/runtime-protocol-websocket-response.test.ts
```

结果为115个非零 selector。新增10个 test声明，`it.each`展开后为12个新 selector，覆盖：

- public/Internal/platform fixed与control unaryFrame v2 exact header/bytes；
- ordinary fixed/control互斥类型与 matching generic control；
- endpoint legacy v1、mixed、fixed空、control非空和malformed payload拒绝；
- fixed unary与stream Assembly HTTP脱敏、generic 5xx details redaction；
- WebSocket connect upgrade、receive close及长 UTF-8 correlation的123-byte边界；
- fixed错误类三种 finite kind只保留correlation。

focused集合：

```text
7 files passed
115 tests passed
```

共享`MockRuntime.sendError`迁移到 v2 control后，另运行两个直接依赖 selector：

```text
tests/test-dispatch.test.ts >
  returns runtime response.error frames without converting them to HTTP errors
  1 passed / 11 skipped

tests/raw-http.test.ts >
  maps serverStream runtime errors before response.start to platform errors
  1 passed / 30 skipped
```

`pnpm --filter @skiff/router run type-check`只剩并行 T 尚未合流的预授权断点：

```text
router/src/router/httpGateway.ts(1007,11):
Property 'visibility' is missing ... but required in type 'TelemetryEvent'
```

F341及其它文件没有 TypeScript错误。`git diff --check`通过。

没有运行完整 Router/workspace/root、stable/live或chat smoke。

## Blocking

F341 blocking：无。

并行 T owner的`httpGateway.ts` visibility改动尚未合流，因此当前分支的 aggregate type-check按任务
预期非零；该断点不属于本 owner，未越界修改。
