# P5-F05：Canonical WebSocket Ingress Authoring

## 输入、owner与顺序

- 依赖：D05完成、F04 narrow receive PASS、R02A第三次PASS；必须在F03A2 shared request writer结束后串行执行，
  完成并通过R05才解锁F03B/F03C。
- 单一高风险owner覆盖std/compiler/deployment、canonical WS request wire、runtime adapter/eval与Router assembly
  gateway；使用独立worktree/branch，只提交一个clean commit，不merge/push。
- 不改变PackageArtifact、ServiceContract、ServiceDeployment、RuntimeAssembly结构，不恢复legacy adapter、不加
  test hook/ambient state、不手改或重签artifact。root lock提交前恢复。

## 冻结ABI

新增正式std builtin：

```text
WebSocketIngressEvent<Context> =
  { tag: "connect", connectRequest: WebSocketConnectRequest }
  | { tag: "receive", receiveEvent: WebSocketReceiveEvent<Context> }
```

canonical operation固定为一个参数名`event`的unary callable：

```text
websocket(event: std.websocket.WebSocketIngressEvent<Context>)
  -> std.websocket.WebSocketConnectResult<Context>?
```

- connect必须返回非null typed accept/reject；receive必须经std WebSocket send capability发送后返回null。
- `Context`只允许exact null或同一ServiceContract拥有的exact nominal ContractTypeId；event与result中的Context
  必须相同。错误参数名/arity、stream、throw、context或return shape在activation前拒绝。
- std/compiler/contract identity对这两个builtin只有一个normalization owner；PackageArtifact boundary projection
  正常产生Available，不允许fixture patch。

## Canonical wire与Router

TS/Rust shared enum新增且只允许canonical assembly WS使用：

```json
{"param":"event","source":{"kind":"websocket.ingressEvent"}}
```

Router connect/receive都发送canonical nested assembly routing、稳定`websocketEntryId`、
`gatewayEntryIdentity`及精确EVENT_ARGS。connect payload为空且只有connectRequest；receive payload由有序、不重叠、
完整覆盖的context/message segments组成。Router不发送`contextExpectation`，runtime从pinned ServiceContract descriptor
推导Context。identity只由service/protocol/operation/canonical selector/EVENT_ARGS组成；同一公开入口跨A/B保持
相同entry id，ABI变化必须改变gateway identity。

`AssemblyWebSocketGateway`保存connect时的snapshot/binding/runtime connection/context；B激活后旧socket receive仍
复用A。direct connection send按service + websocketEntryId + connectionId精确路由；business identity fan-out、
connection policy与close清理复用generic gateway的production owner，不得静默忽略。

## Runtime执行

canonical path固定为：assembly lookup → pinned boundary descriptor → materialize typed event →
`dispatch_in_process_boundary` → phase-specific response projection；不得调用legacy/direct-call WS handler。

- request trust boundary要求unary、EVENT_ARGS exact、无contextExpectation，connect/receive元数据与kind严格互斥。
- connect materialize `{tag:"connect", connectRequest}`；receive从payload/context codec materialize
  `{tag:"receive", receiveEvent:{connection,message}}`。typed Context即使编码为零字节也保留segment presence。
- connect null、receive non-null、receive response payload或connect metadata缺失/多余都fail closed。
- reject与accept-null context均为空payload、`contextPayloadPresent:false`、无codec；nominal Context accept携带exact
  payload、presence true与`{operationAbiId, contextTypeIdentity}`。
- fixture receive使用`event.receiveEvent.connection.id`调用`sendTextToConnection(generationMarker)`，不得用
  business fan-out替代generation-pin证明。

## 写入边界与风险探针

允许修改std WebSocket builtin、compiler type/lowering/boundary projection、contract identity normalization、
deployment WS eligibility、canonical request TS/Rust enum/corpus、runtime linked type/boundary/eval/request adapter、
Router assembly WS gateway/identity及其直接tests；F04合流后的real ecosystem smoke只可改为消费production ABI。
不得修改activation/store、HTTP/server-stream语义、F03B统一endpoint/F03C startup owner或外部repo。

首个风险探针必须先用正常contract-first source证明ABI得到Available projection、contract/deployment exact match；
失败即报告compiler/contract blocker，不先铺Router/runtime fallback。

## 聚焦验证

```bash
cargo test -p skiff-compiler-projection websocket_ingress
cargo test -p skiff-deployment websocket_ingress
cargo test -p skiff-runtime-transport runtime_assembly_request_start
cargo test -p skiff-runtime-eval websocket_adapter
cargo test -p skiff-runtime-request websocket_ingress
pnpm --filter @skiff/router type-check
pnpm --filter @skiff/router test -- tests/host-ingress.test.ts tests/websocket-gateway.test.ts tests/protocol.test.ts
node cross-system-fixtures/package-service-ecosystem/verify.mjs --runtime-wire-self-test
git diff --check
```

每个filter必须非零。正例覆盖A连接/receive marker A、激活B、新unary/WS marker B、旧连接再次marker A并自然关闭；
负例覆盖错误ABI/context/segment/identity、跨service/entry send、drain后连接与新请求误入旧generation。回报
source/commit/tree、ABI→wire→runtime→send矩阵、single commit/clean/lock状态与残余风险。
