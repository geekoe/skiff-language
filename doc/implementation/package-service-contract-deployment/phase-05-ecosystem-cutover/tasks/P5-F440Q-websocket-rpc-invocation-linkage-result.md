# P5-F440Q WebSocket RPC invocation linkage result

状态：`PASS / T0_SCOPED_IMPLEMENTATION_VALID`。

`std.websocket.requestJsonToConnection` 现在由当前 linked call plan携带 exact
`std.websocket.WebSocketRequestError` named-union owner，并由当前
`RuntimeNativeInvocation`消费；owner不再位于可跨调用复用的 capability对象。Host production
execution同时附着 captured Router session/writer、共享`ConnectionRequestRegistry`、ancestor
cancellation与当前 effective execution deadline。F440P冻结的wire、registry、五分支ordinary error
carrier和默认`Unsupported`均未改语义。

本节点没有创建inbound JSON-RPC adapter、target、broker或gateway模块；因此是T0完整实现检查点，
不是E0/R0稳定候选。

## 1. 基线与提交

| 状态 | Commit | Tree |
| --- | --- | --- |
| 任务声明的 implementation baseline | `c2abd2e84d7d1ff9ac3f018c67c00518f890c3dd` | `3389741b6ee927cf97c4943130e0fdc29af5af82` |
| worktree实际起点 | `b99bfd40df25bb538e73b178bfa3d5645661c322` | `1febd5d6c575d34f7682ba7694911bf6af565d06` |
| implementation | `07157c63ae3e85199b8027fd2979c88e1a1ff7dc` | `ced8d7cb76ba04141cef31442e2edff57b778092` |

`b99bfd40`直接基于任务声明的`c2abd2e8`，中间只增加F440Q/F440R调度文档。
Implementation与本文result分离提交；result commit/tree由最终交付消息记录。

## 2. Exact owner与invocation边界

### 2.1 Call plan owner

`runtime/native-contract/src/call_plan.rs`为`NativeCallPlan`增加私有、可选的
`NamedUnionOwnerIdentity`：

- 只有exact binding key `std.websocket.requestJsonToConnection`可以设置；
- 其它native尝试设置owner立即返回错误，不会获得伪owner；
- plan默认没有owner，保留未解析artifact的fail-closed状态。

`runtime/linked-type-plan/src/native_call_plan.rs`只为该binding解析owner。解析同时要求：

1. 当前linked program存在exact package id `skiff.run/std`及其exact package slot；
2. exact public path `std.websocket.WebSocketRequestError`同时存在于admitted type export与当前
   executable link overlay；
3. overlay symbol必须是与export相同`TypeAddr`的`ResolvedSymbol::Type`；
4. declaration必须是同名、non-generic `LinkedTypeDescriptor::Union`。

缺package/export/overlay、overlay错误kind、overlay/export地址冲突和非union declaration都返回
`InvalidArtifact`。结果是当前program的
`NamedUnionOwnerIdentity::LocalExecution(LocalExecutionTypeIdentity { addr, ... })`，没有global
lookup、固定地址、platform builtin或跨caller cache。

### 2.2 Invocation与native dispatch

`runtime/native/src/dispatch/invocation.rs`从当前call plan暴露required exact owner；缺失时返回
`InvalidArtifact`。`runtime/native/src/dispatch/websocket.rs`在本地codec和任何Host/peer side
effect之前先验证owner，然后只从invocation构造F440P的
`WebSocketRequestError`。

`NativeWebsocketCapability`上的owner getter已经删除。capability仍只拥有transport行为，不能猜测
调用者类型身份。`runtime/eval/src/native_invocation.rs`无需复制字段：它构造的
`RuntimeNativeInvocation`已经持有本次解析得到的完整`NativeCallPlan`。

五个ordinary terminal仍精确投影为同一owner下的：

```text
connectionUnavailable
transportUnavailable
protocolError
resourceLimit
remote
```

其中branch identity继续使用`kind` discriminator。local request encode失败仍是
`std.json.encode`的decode error路径；deadline仍是`TimeoutError`对应的
`ExecutionBudgetExceeded::DeadlineExceeded`；ancestor cancellation仍为不可捕获的
`RuntimeError::Cancelled`。

## 3. Host production attachment

### 3.1 Eval-local request extension

F440P共享`runtime/capability-context`不在本节点写集内，因此
`runtime/eval/src/capabilities.rs`在既有raw WebSocket capability外增加eval-local request API：

- 默认构造继续返回`Unsupported("...execution is not attached")`；
- attached构造持有独立request API；
- `owned()`、`borrow()`和provider activation rebinding都保留同一invocation transport事实；
- `RuntimeNativeWebsocketCapabilityContext`只转发request future并执行既有eval/native error映射。

这没有把owner放回capability；owner只随`RuntimeNativeInvocation`传递。

### 3.2 Captured Host facts

`runtime/host/src/eval_capability_adapter/{websocket.rs,factory.rs}`把以下事实冻结在当前execution：

- Host共享`Arc<ConnectionRequestRegistry>`；
- exact `ConnectionRequestSession`；
- captured `RouterWriterMessage` sender；
- current ancestor `CancellationToken`；
- current effective monotonic execution deadline及对应positive safe-integer/RFC3339 wire control；
- 当前activation/provider的service id与exact WebSocket entry id。

当前public native ABI没有额外per-call timeout policy，因此strict effective result就是该execution
已经收敛后的deadline；接线没有自行增加或延长policy。

attached request调用F440P既有
`concrete::WebsocketCapabilityContext::with_request_transport(...)`，因此继续由原registry保证
install-before-queue、session fence、single settlement、late/duplicate rejection以及
cancel-before-best-effort-wire的顺序。没有复制或改变wire/registry算法。

HTTP gateway与WebSocket connect两条既有Host execution入口都附着上述事实；provider activation
rebinder只替换service/entry route，继续使用原invocation的registry/session/writer/cancel/deadline。

Host测试覆盖success、remote、ancestor cancel、deadline、disconnect五条production attached
生命周期。每条结束时都断言：

```text
pending_count == 0
active_lease_count == 0
active_timer_count == 0
```

cancel/deadline还断言专用`connection.request.cancel`及exact reason；disconnect断言原session
settle为`TransportUnavailable`且late response不能重开pending。

## 4. 旧evaluator显式fail closed

`runtime/eval/src/runtime_http_gateway.rs`新增显式拒绝：

- `GatewayAdapterKind::WebSocketJsonRpc`；
- `GatewayAdapterSource::{WebSocketJsonRpcParams, WebSocketBusinessIdentity}`。

`runtime/eval/src/runtime_websocket_connect.rs`同样显式拒绝两个JSON-RPC-only source。两处均返回
invalid target/source protocol error，不使用`unreachable!`、panic、空值或raw HTTP/connect参数
替代。colocated tests证明拒绝发生在value projection/handler execution之前。

没有增加`runtime_websocket_jsonrpc*`文件或inbound evaluator。

## 5. Test-first red证据

在production实现前先增加：

```text
cargo test -p skiff-runtime-native-contract \
  websocket_request_plan_alone_carries_exact_linked_error_owner
```

修正测试自身fixture构造后，真实red为exit `101`：编译器分别报告
`NativeCallPlan::with_named_union_error_owner`与
`NativeCallPlan::named_union_error_owner`不存在。此时production没有owner字段或访问路径。

最小实现后同一selector得到`1 passed / 5 filtered`。随后才接入linked resolver、invocation、
native dispatch和Host future。

完整selector编译还暴露三类真实遮罩：

- eval测试fixture仍返回旧共享WebSocket context，而production已使用eval-local wrapper；
- current model已经删除`PlatformBuiltinErrorIdentity::Cancel`，但旧negative test仍直接构造该variant；
- S0 model要求`GatewayWebSocketConnectProtocolSurface.rpc_profiles`，两个旧test fixture未填写。

这些只做第7节列出的机械compile-mask修补，没有扩展production协议。

## 6. Green验证

所有Cargo命令统一使用：

```text
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target
```

| 必跑命令 | 实际执行 | 结果 |
| --- | ---: | --- |
| `cargo test -p skiff-runtime-native-contract` | 6 | PASS：6 passed；0 doctests |
| `cargo test -p skiff-runtime-linked-type-plan native` | 7 | PASS：7 passed / 13 filtered |
| `cargo test -p skiff-runtime-native websocket` | 8 | PASS：8 passed / 93 filtered |
| `cargo test -p skiff-runtime-eval native` | 10 | PASS：10 passed / 199 filtered；两个integration binary均0执行且各有4/6 filtered |
| `cargo test -p skiff-runtime-host connection_request --no-fail-fast` | 8 | PASS：8 passed / 282 filtered；三个integration binary均0执行且各有2/6/2 filtered |
| `cargo check -p skiff-runtime-host` | — | PASS |
| `cargo fmt --all -- --check` | — | PASS |
| `git diff --check` | — | PASS |

Host selector的8项包括：

- 默认unattached request保持`Unsupported`且不发frame；
- concrete F440P future install-before-queue与opaque success；
- production captured session/provider route success；
- production remote terminal；
- production ancestor cancel；
- production deadline；
- production disconnect/session fence；
- router response exact-session demux。

linked/native selectors另外证明：

- exact current linked owner；
- 两个不同linked program/caller的owner不交叉复用；
- missing、wrong-kind、address conflict及non-union owner fail closed；
- missing owner在capability dispatch计数仍为0时失败；
- 五个branch的exact catch identity；
- local codec、deadline与ancestor cancel分流。

最终命令只有现有unused/dead-code/unreachable-pattern warnings，没有test/check失败。

## 7. Mechanical call-site与compile-mask范围

父任务确认以下是合同中“F440P Host connection-request tests所需机械call-site”的窄例外：

- `runtime/host/src/host/request_entry/assembly_wire.rs:39,62,71`：把既有
  `router_session_id`参数原样传给HTTP/WebSocket两条既有spawn路径；
- `runtime/host/src/host/request_entry/assembly.rs:195,222,461,485-487,564,588-590`：
  只把captured session解析为`ConnectionRequestSession`并把Host既有registry传到capability
  construction；
- `runtime/host/src/eval_capability_adapter/assembly_request_adapter.rs:31-32,155-156,318-326,373-380`：
  在既有execution construction中机械携带registry/session，并读取该execution已有的
  cancellation/deadline。

这些文件没有新增target、wire shape、router branch、routing选择或loader production behavior。

另有仅为必跑selectors解除current-tree compile mask的测试更新：

- `runtime/host/src/loader/active_assembly_context.rs:593`：tests-only WebSocket surface fixture补
  `JsonRpc2_0Text` profile；
- `runtime/eval/src/runtime_http_gateway/tests.rs:281`：同一tests-only fixture字段补齐；
- `runtime/eval/src/assembly_execution/ordinary/test_runtime.rs:29,850-854`：测试double的trait
  返回类型继续使用共享raw context，测试execution helper改用eval-local wrapper；
- `runtime/eval/src/assembly_execution/service_error_channel/tests.rs:126-129`：删除对已不存在
  `Cancel` enum variant的直接构造，改为断言legacy cancellation symbol不在finite platform registry。

## 8. 范围与停止条件审计

Implementation共修改`18`个文件。除第7节经确认的机械例外外，均位于任务唯一写集。

明确未修改：

- `std/` public ABI；
- artifact gateway schema与native signature；
- F440P connection wire、registry、Router demux语义；
- `runtime/request`；
- Router、broker、gateway/server；
- fixture、test-runner、scripts、README及其它task/result；
- 任何inbound E0模块。

Host outbound attachment不需要E0尚不存在的inbound target/API，因此没有触发
`TASK_SCOPE_EXPANDED`。未运行complete verify、live、stable、instance、watch或chat smoke；未派
子agent；未merge、rebase或push。
