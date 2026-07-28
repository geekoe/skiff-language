# P5-F445H-D2 WebSocket peer-cancel hard cut

状态：Ready。D1已经冻结“第一版没有公开取消请求或WebSocket peer cancel协议”。本节点只删除Router
JSON-RPC profile/broker仍在执行的`$/cancelRequest`与`-32800`路径；runtime内部stop frame继续作为
best-effort资源提示存在。该节点与O6R evaluator DB写集互不重叠，可以并行。

## 直接父节点

- `P5-F445H-D1-internal-execution-stop-semantics-result.md`
- `P5-F440R2-router-rpc-core-responsibility-split-result.md`
- `P5-F440Z3E-router-websocket-rpc-gateway-integration-resume-result.md`

production prerequisite为integration commit `d6a6ca36`。

本任务文件完整描述执行需求。D1拥有当前公开语义；两个F440 result只提供现有profile、broker、bridge和
gateway owner事实。不得重写内部request stop、durable queue、Actor、spawn或其它取消语义。

## 当前owner与缺口

现有唯一peer JSON-RPC实现位于：

- `router/src/protocol/jsonRpc20TextProfileContracts.ts`
- `router/src/protocol/jsonRpc20TextProfileImplementation.ts`
- `router/src/router/webSocketRequestBroker.ts`
- `router/src/router/webSocketRequestBrokerWire.ts`

当前仍有四条已过时路径：

1. profile把合法`$/cancelRequest` notification分类为专用`ProfileAction.kind = "cancel"`；
2. profile可以编码peer cancel notification，并把`cancelled`映射为
   `-32800 Request cancelled`；
3. outbound runtime stop/deadline/runtime disconnect会best-effort向peer写cancel notification；
4. inbound peer cancel会abort对应Skiff handler并写`-32800`。

真实gateway只通过`WebSocketRpcBridge`调用同一个broker。`connection.request.cancel`和
`request.cancel`是runtime内部frame，分别由bridge/dispatcher接收；本任务不得删除这些frame或
改变其strict schema。它们在本节点之后只造成Router本地pending detach、handler内部停止或late-result
隔离，不再投影成peer协议。

上游遮挡关系：

```text
profile仍识别专用cancel
  -> broker才能创建peer cancel terminal
  -> bridge/gateway测试继续锁定旧公开协议

broker仍编码cancel
  -> runtime内部stop被错误投影到外部wire
```

## 生产目标

### 1. Profile hard cut

从public profile contract和唯一implementation删除：

- `PlatformRpcError`的`cancelled`分支；
- `ProfileAction`的`cancel`分支；
- `WebSocketRpcProfileAdapter.encodeCancel`；
- `JsonRpc20TextProfile.encodeCancel`；
- cancel-specific params parser；
- `-32800 Request cancelled`常量/编码。

没有`id`、`method`合法且`params`缺失或为object/array的frame统一按普通
`ignoredNotification`处理，method是否为`$/cancelRequest`或其它平台前缀都没有特殊意义。
Notification若在JSON-RPC request-object层面本来就畸形，继续沿既有invalid-request/parse/close规则；
不要为该method保留专用params校验。

有`id`的`{"method":"$/cancelRequest", ...}`不是notification，继续作为普通peer request按声明的
method table处理；不得偷偷把method名字保留为平台控制入口。

### 2. Broker hard cut

删除：

- `tryEncodePeerCancelFrame`；
- inbound `handleInboundCancel`；
- outbound `bestEffortCancel`；
- `settleOutbound(... cancelPeer ...)`及相应分支。

保留但收窄现有内部入口：

- `handleRuntimeCancel`先原子detach本地outbound pending、写bounded tombstone，然后直接返回；不向peer
  写任何frame，也不向已经结束的runtime caller发送第二个terminal；
- outbound deadline先detach，再向runtime返回一次`deadlineExceeded`；不向peer写cancel；
- runtime source disconnect只detach相关pending；不向peer写cancel；
- peer/socket disconnect、protocol close、deadline等既有本地AbortController仍可内部停止inbound
  handler，但不能生成cancel error或rollback承诺；
- 晚到/重复response继续只命中bounded tombstone，不得恢复已经结束的调用。

所有有效notification只走`ignoredNotification`诊断回调，不dispatch、不response、不abort active
request。Observer抛错仍不得影响broker状态。

### 3. README

同步更新`router/README.md`与`runtime/README.md`：

- 所有peer notification均被忽略；
- 没有`$/cancelRequest`例外或`-32800`；
- broker拥有deadline、内部停止和settled state；
- runtime内部stop只做本地资源收束，不是peer cancellation。

## 测试与完成标准

先修改测试形成真实RED，再改production。聚焦测试至少覆盖：

1. 合法`$/cancelRequest` notification返回`ignoredNotification`，不再要求内部`params.id`；
2. 其它合法notification行为不变；畸形notification仍按既有协议规则处理；
3. profile contract/implementation不能编码`-32800`或peer cancel frame；
4. runtime内部cancel会detach outbound pending、写tombstone且writer计数不增加；
5. outbound deadline只向runtime返回一次`deadlineExceeded`，peer writer没有cancel frame；
6. runtime source disconnect不写peer cancel；
7. peer `$/cancelRequest` notification不会abort active inbound handler；handler随后成功时仍只写一次
   normal result；
8. unknown/already-settled notification同样无状态变化或response；
9. 两个方向使用同值id仍隔离；有效notification不能误命中outbound response或inbound request；
10. bridge真实调用路径保持上述行为，generation/pending/timer/tombstone accounting归零；
11. socket disconnect、deadline、capacity、protocol error和late completion既有行为不回归；
12. raw WebSocket send与普通JSON-RPC request/response wire不变。

主要测试owner：

- `router/tests/json-rpc-20-text-profile.test.ts`
- `router/tests/websocket-request-broker.test.ts`
- `router/tests/websocket-rpc-bridge.test.ts`

若现有同路径测试位于其它Router test文件，可作机械跟随；不得扩大到Rust production或重新设计
gateway。

开发Agent拥有以下验证：

```bash
router/node_modules/.bin/vitest list --root router \
  tests/json-rpc-20-text-profile.test.ts \
  tests/websocket-request-broker.test.ts \
  tests/websocket-rpc-bridge.test.ts
router/node_modules/.bin/vitest run --root router \
  tests/json-rpc-20-text-profile.test.ts \
  tests/websocket-request-broker.test.ts \
  tests/websocket-rpc-bridge.test.ts
pnpm --dir router type-check
git diff --check
```

记录listing与execution的非零测试数。Worktree若没有依赖，只允许临时链接主仓库已有
`router/node_modules`和必要root `node_modules`；不得安装或访问网络，完成后必须删除临时链接。
不得运行stable/live、启动长期server或执行阶段完整gate。

反向搜索要求：

- production和README中`$/cancelRequest`、`-32800`、`Request cancelled`为零；
- profile production中`cancelled` error、`ProfileAction.kind = "cancel"`、`encodeCancel`为零；
- broker production中`cancelPeer`、`bestEffortCancel`、`tryEncodePeerCancelFrame`为零；
- 测试中若保留`$/cancelRequest`，只能作为“普通有id request”或“被忽略notification”的负例；
- internal `request.cancel`、`connection.request.cancel`、`RequestCancelReason`和durable
  queue/Actor/spawn cancellation不得被误删。

## 写入范围与停止条件

允许：

- 上述四个Router production owner；
- 三个主要Router测试及同路径机械测试跟随；
- `router/README.md`
- `runtime/README.md`
- 本result

不得修改Runtime/Rust production、runtime transport schema、`RuntimeEndpoint`、
`RuntimeDispatcher`、`WebSocketRpcBridge` production、gateway、compiler、artifact、manifest、
lockfile或权威设计。

若删除peer cancel需要改变internal frame schema、runtime terminal enum、public std API或写集外
production owner，完成一次有界探查后返回`TASK_SCOPE_EXPANDED`；不得顺手删除所有内部cancel命名。
若五分钟内仍不能形成第一处实际测试修改，返回`TASK_NOT_EXECUTABLE`。

风险：高（外部wire hard cut）。完成后仍只是实现检查点；必须在后续稳定候选上由独立验收owner检查
profile→broker→bridge真实路径。

## Worktree与交付

```text
worktree /Users/geek/workspace/skiff-p5-f445h-d2-websocket-stop
branch   codex/p5-f445h-d2-websocket-stop
```

先提交implementation，再单独提交
`P5-F445H-D2-websocket-peer-cancel-hard-cut-result.md`。最终worktree clean；不得merge、rebase或
push。

这是一次性有界开发会话。当前任务不需要子Agent；若有范围扩张或多个不明确问题，按工作流停止并
如实上报。

