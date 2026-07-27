# P5-F429A Runtime/Host current WebSocket connect execution result

状态：`TASK_SCOPE_EXPANDED / SAFE CHECKPOINT`。授权范围内的 current connect admission、
typed activation entry、exact callable execution、current response mapping和顶层 activation capability
接线已经形成可编译检查点；但任务第8项要求删除的 legacy shape 仍被两个范围外 production owner
重导出或穷尽匹配。发现该合同级越界后按停止条件不再修改实现，也没有把本结果判为完成。

## 1. Exact candidate 与 implementation checkpoint

- frozen input / tree：
  `1f52b2f5053830134e59bfa6f5c67d787078efa2` /
  `d859b21fbbbf8c1c3db724af53ebf3654e0c3a94`；
- task checkout / tree：
  `3769eec5cc8599dbe1a54833eae9fcd00545c589` /
  `6f868363f97266fd3735c99bf8c4f1ce688f9645`；
- implementation checkpoint / tree：
  `2b8897ab75aee674afbd6438a38af03b16556751` /
  `3a1f907f666a6f30fbd73f34f3ed23fd960253f0`。

task checkout相对 frozen input只增加任务合同文档。implementation checkpoint共修改授权范围内
14个Rust文件；没有修改 frozen F426A current wire/corpus、Router、compiler、deployment、
test-runner或ecosystem仓库。

## 2. 授权范围内已形成的安全检查点

### 2.1 Admission 与 activation exact record

- `active_assembly_context::admitted_websocket_entry`从exact linked deployment解析零或一个
  WebSocket gateway entry/binding；
- admission检查compiler-owned entry key、canonical selector、gateway identity、
  `GatewayEntryProtocolSurface`、adapter kind/args、handler/pre/guard和canonical
  `WebSocketEntryId`的exact join；
- `ActivationContext`保留私有typed optional sole-entry record，并只暴露typed id和exact
  match查询；现有constructor明确传入`None`，没有空字符串或默认entry回填。

### 2.2 Current connect execution

- Host request entry按F426A closed union分发HTTP或`websocketConnect`，并对空payload、
  ingress selector、assembly identity/generation、gateway identity、internal entry id和activation
  record做exact验证；
- `RuntimeAssemblyWebSocketConnectTarget`只接受exact private linked callable，验证owner、
  execution image、callable id/target/signature、protocol surface和adapter plan；
- eval只从connect request与connection id构造handler参数，不经过Context、receive/message或
  service operation lookup；
- non-generic accept/reject结果通过专用current mapper编码为F426A
  `RuntimeAssemblyWebSocketConnectResponseEndFrameHeader`，payload固定为空；
- 无handler dispatch在进入eval前fail closed；Runtime不合成accept；
- accept在响应前获取exact service/entry/connection generation pin；reject和generic error不获取。

### 2.3 顶层 capability 接线

- HTTP、connect和actor顶层执行context都从activation sole-entry构造WebSocket native
  capability；
- 零entry沿现有capability factory生成unavailable状态，不用caller数据补齐entry；
- 四个native public签名、`may_suspend=false`和control frame的service + entry字段未改变。

## 3. 自验收矩阵

| 合同项 | checkpoint状态 | 证据或剩余项 |
| --- | --- | --- |
| 1. sole-entry admission exact join | IMPLEMENTED / UNTESTED | admission helper与activation注入已编译；聚焦正负测试因scope stop未运行 |
| 2. typed optional activation record | IMPLEMENTED / UNTESTED | typed private record；零entry不默认填充 |
| 3. current connect Host dispatch | IMPLEMENTED / UNTESTED | F426A header与activation exact match；handler required |
| 4. non-generic accept/reject | IMPLEMENTED / UNTESTED | dedicated eval decoder和current wire mapper |
| 5. no-handler由Runtime拒绝 | IMPLEMENTED / UNTESTED | target construction与Host dispatch双重fail closed |
| 6. 四类activation native capability | PARTIAL | HTTP/connect/actor顶层完成；ordinary nested service call仍继承caller capability，见第5节 |
| 7. outbound service + entry / generation | IMPLEMENTED CHECKPOINT / UNTESTED | connect accept generation acquire与activation-derived capability已接线；未跑targeted lifecycle tests |
| 8. 删除D2 legacy consumer/shape | BLOCKED / TASK_SCOPE_EXPANDED | 两个范围外production owner使授权内删除无法编译闭合，见第4节 |

因此本checkpoint不解除F429A，不允许F429B合流后把combined probe判为PASS。

## 4. TASK_SCOPE_EXPANDED：两个新增 production owner

### 4.1 `runtime/driver/eval/mod.rs`

精确范围外symbols：

- `eval::websocket_adapter` module shim：
  `pub(crate) use skiff_runtime_eval::websocket_adapter::*`；
- `skiff_runtime_eval` named re-exports：
  `EvalRequestInvocationWebSocketAdapter`、
  `EvalRequestInvocationWebSocketConnectRequest`、
  `EvalRequestInvocationWebSocketContextCodec`、
  `EvalRequestInvocationWebSocketContextExpectation`、
  `EvalRequestInvocationWebSocketKind`、
  `EvalRequestInvocationWebSocketMessage`、
  `EvalRequestInvocationWebSocketMessageEncoding`、
  `EvalRequestInvocationWebSocketMessageTag`、
  `EvalRequestInvocationWebSocketNameValue`、
  `EvalRequestInvocationWebSocketPayloadSegment`、
  `EvalRequestInvocationWebSocketPayloadSegmentKind`、
  `EvalRequestInvocationWebSocketReceiveRequest`、
  `EvalRequestWebSocketAdapterResult`、
  `EvalRequestWebSocketConnectAccept`、
  `EvalRequestWebSocketConnectContext`、
  `EvalRequestWebSocketConnectReject`和
  `EvalRequestWebSocketContextCodec`。

精确重导链：

```text
runtime/eval/src/{websocket_adapter.rs,invocation.rs,invocation_builder.rs}
  -> runtime/eval/src/request_boundary.rs legacy aliases
  -> runtime/eval/src/lib.rs public module / named exports
  -> runtime/driver/eval/mod.rs wildcard module shim / named re-exports
```

删除任务已授权的eval D2 owners后，该driver shim产生unresolved import；保留shim又违反第8项的
production reverse-search要求。这是机械闭合而不是新语义：driver内没有该shim或这些named
WebSocket aliases的调用者，后继只需删除已失去source owner的重导出。

若同时删除`runtime/request-contract::RequestEnvelope.websocket_adapter`，两个范围外direct test
fixture还需机械删除`None`字段初始化：

- `runtime/driver/eval/eval_context/tests.rs::test_request`；
- `runtime/driver/eval/tests/program_execution.rs::test_invocation`。

它们不实现WebSocket行为，只是完整struct literal的编译闭合点。

### 4.2 `runtime/host/src/host/http_response_ceiling.rs`

精确范围外symbols：

- production `validate_unary_response`中的
  `BoundaryResponse::Event(ResponseEvent::End(ResponseEnd::WebSocket(_)))` match arm；
- embedded test
  `non_http_and_websocket_responses_do_not_consume_http_budget`中的
  `BoundaryResponse::websocket`、`WebSocketResponse::ConnectAccept`和
  `WebSocketConnectContext::Null`。

精确调用/重导链：

```text
runtime/request-contract/src/response_event.rs
  ::{BoundaryResponse::websocket,ResponseEnd::WebSocket,
     WebSocketResponse,WebSocketConnectContext}
  -> runtime/request-contract/src/lib.rs exports
  -> runtime/request/src/lib.rs re-exports
  -> runtime/host/src/host/http_response_ceiling.rs
     ::validate_unary_response exhaustive match + embedded test
```

删除任务已授权的request-contract/request legacy response shape后，该Host exhaustive match和test
无法编译。删除它们同样是机械闭合而不是新语义：current connect响应由
`runtime_assembly_websocket_connect_response_into_frame`直接编码，不再进入
`BoundaryResponse`或HTTP response ceiling；HTTP payload计数逻辑保持不变。

## 5. 原授权范围内仍需完成的 ordinary service-call capability rebind

scope stop前的只读审计还确认任务第6项尚有一个原授权范围内缺口：

```text
runtime/eval/src/assembly_execution/async_stream_cancel.rs
  ::provider_execution_context
  -> receiver.clone()
  -> with_runtime_assembly_target(provider_target)
```

`ProgramExecutionContext::clone`会复制caller的`websocket` capability；随后
`with_runtime_assembly_target`只替换assembly eval target和stream scope，没有按provider
activation的sole-entry重建WebSocket capability。因此普通in-process service call进入provider
activation时仍可能携带caller service/entry。后继必须在既有F429A授权的eval/Host capability
范围内加入provider activation capability rebinder，并覆盖caller/provider entry不同与provider
零entry的direct tests。当前checkpoint不声称第6项完成。

## 6. 最小新增任务写集

为机械闭合第8项，最小新增production写集是：

```text
runtime/driver/eval/mod.rs
runtime/host/src/host/http_response_ceiling.rs
```

并只在删除`RequestEnvelope.websocket_adapter`导致struct literal失配时，加入两个direct fixture：

```text
runtime/driver/eval/eval_context/tests.rs
runtime/driver/eval/tests/program_execution.rs
```

新任务不需要修改wire、Router、gateway policy或公共std签名。它应在上述机械闭合后恢复本任务原
write set，完成D2 legacy删除、ordinary service-call capability rebind和全部聚焦测试；不能只留
compatibility alias来绕过reverse-search。

## 7. 已通过证据与未运行门禁

implementation checkpoint格式化后实际通过：

| 命令 | 结果 |
| --- | --- |
| `cargo fmt --all -- --check` | PASS |
| `git diff --check` | PASS |
| `cargo check -p skiff-runtime-host` | PASS；只有既存dead-code warnings，并连带type-check activation/eval/request/transport/host依赖链 |

按照`TASK_SCOPE_EXPANDED`停止条件，没有运行或伪报任务规定的combined package tests，也没有形成
sole-entry admission、connect accept/reject、无handler、ordinary HTTP/service native或generation
lifecycle的动态证据。完整验证矩阵必须由获得新增write set的后继在同一exact candidate上执行。

## 8. Scope 与生命周期

- 没有修改两个范围外owner或其direct fixtures；
- 没有修改F426A wire/corpus、Router、compiler/authoring/deployment producer、test-runner、
  Internals或skiff-packages；
- 没有merge、rebase、push、stable/live/instance操作；
- implementation与本result分开提交；
- 本结果只交付safe checkpoint和精确扩张合同，不解除F429A、D4或combined probe。
