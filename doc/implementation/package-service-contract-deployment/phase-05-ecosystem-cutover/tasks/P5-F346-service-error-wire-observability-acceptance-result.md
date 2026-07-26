# P5-F346 Service error wire / observability independent acceptance result

状态：`PASS`（W2-W / C1 / S1 高风险边界独立只读验收通过；无 blocking；不代表整个
Phase 05 完成）。

## Exact candidate 与只读边界

- candidate branch：`codex/package-service-phase-05`
- exact candidate commit：
  `d4b5501d32bfdcbb8d7026421bb026e97575b904`
- exact candidate tree：
  `94d310fecbb2c2a713eb88dbee4b29bcf2fae5f4`
- F345 merge commit：
  `191e71a727b456fbcd4ca32d5484692138381190`
- F345 merge tree：
  `e3a3443c874f2737ba4dfc28adad4b579be691a3`
- acceptance worktree：
  `/Users/geek/workspace/skiff-p5-f346-service-error-acceptance`
- acceptance branch：
  `codex/p5-f346-service-error-acceptance`

`191e71a7`是 exact candidate 的祖先。`191e71a7..d4b5501d`只有 F345 task 状态更新和新增
F346 task；任务列出的
`runtime/{model,boundary,eval,capability-context,request,request-contract,transport,host}`、
`router/src`、`telemetry/src`、C0 corpus和 C1 scenario fixture均无 diff。

验收未修改 production、tests、fixture、corpus、lockfile、设计或 task 状态，未安装或更新依赖，
未运行 stable/live/Mongo live，未 push。Router与Telemetry验证只临时复用本机已有、与各自
lockfile匹配的依赖目录；链接在验证后删除。唯一交付写入是本 result。

## 独立验收结论

### 1. 固定错误语义

结论：`PASS`。

- `PackageCallableSignature`只有 parameters/return/maySuspend；
  `BoundaryOperationContract`只有 parameter/return/stream/cancellation/callback/effect事实；
  `ServiceContract`只引用 operation descriptor与 package type requirements。三者的 strict serde
  均拒绝旧 `throwTypes`或 operation `errors`。throw进入 request-local
  `RequestException`只要求真实 catch identity，不读取 operation-specific throws set。
- `CanonicalServiceErrorChannel::export_local_exception`只在类型公开可命名、无开放类型参数且
  schema plan可编码时构造`PublicTypedError`。owner/key/type-id直接取自 exact
  `ServiceErrorTypeLink::public_identity()`，并校验其 Package build/schema closure；
  dependency Package类型不会被重写为 provider service owner。
- private、不可公开命名、非 closed、`std.service.InternalError`本地身份或编码失败均进入
  `fixed_internal`，只含固定 message `Internal service error`与既有 correlation。fixed wire没有
  原 identity、字段、display或 callee stack；restricted sidecar也只含有限 cause、typed
  source/stack及 owner/correlation，不持有 heap value或 payload。
- 已经 fixed 的`RuntimeError::FixedServiceFailure`或 imported
  `RequestException`在下一 hop直接 clone原`OpaqueServiceError`，不会重新生成 envelope或
  correlation。caller import从新的 caller-local stack开始，只追加一条
  `RemoteBoundary{serviceId,operationId,errorId}`；校验还拒绝 caller输入中预存的 remote frame。
- 真实 ordinary 三跳 selector同时覆盖 public和 private失败：terminal/relay各提交一次
  provider-local diagnostic，B/A各 import一次；两次 import bytes相等，最终 carrier仍相等，
  每个 caller只有自己的 local frame和一条 remote-boundary frame。

### 2. wire / Host / Router / gateway

结论：`PASS`。

- Rust `ResponseErrorFrameHeader`是
  `#[serde(tag="errorKind", deny_unknown_fields)]`的严格
  `fixedService | control` union；两个 constructor都固定
  `skiff-runtime-frame-v2`与`response.error`。Router interface、declarative `oneOf`和 manual
  validator镜像同一 exact field set。生产中没有 v1 response.error reader、writer、fallback或
  code/message升级逻辑。
- fixed必须有非空 payload并由`OpaqueServiceError::decode`/Router strict envelope validator
  接受；control必须空 payload并有 nonblank code/message及合法 status。C0 corpus仍为单一
  `runtime/transport/testdata/service-error-response-v2.json`，当前包含4个 valid和30个 invalid
  case；legacy v1、unknown/missing discriminator、mixed field、payload presence、unknown/extra
  envelope字段和 malformed payload均在 invalid集合。
- Host在调用`RequestError::response_error()`前先执行 typed extraction。fixed进入
  `ResponseEvent::FixedServiceFailure`与`complete_fixed_service_failure`；control才进入
  `ResponseEvent::Error`、generic response payload和`complete_error`。
- Rust encoder使用`OpaqueServiceError::into_encoded_bytes()`写 fixed binary payload；
  dedicated decoder保留收到的 bytes；reverse session恢复
  `OutboundResponse::FixedServiceFailure`。Router admission只调用
  `validateResponseErrorFrame`，`RuntimeDispatcher`的 unaryFrame直接返回同一个 header和
  `Uint8Array`对象，ordinary pending按 union显式选择 fixed/control mapper。没有 stringify、
  envelope re-encode或按 code/message/status/shape分类 fixed。
- matching `InternalError` code/message的 control仍为
  `RuntimeResponseError`；fixed只成为`FixedServiceResponseError`。后者只保留有限 kind及
  traceId/errorId，不保存 raw envelope、encoded payload或 diagnostic。
- production Assembly HTTP写`toHttpPayload()`；fixed输出稳定
  `FixedServiceError / Service request failed`和 correlation。production Assembly WebSocket
  upgrade与 close reason共用`toExternalMessage()`，并对 close reason执行123-byte UTF-8边界截断。
  实际 gateway抽查的 raw HTTP/WS结果不含 private sentinel、payload、source/path/function、
  frames或stack。

### 3. observability

结论：`PASS`。

- fixed operational completion只投影有限
  `{kind: fixedService, causeKind: publicTypedError|internalError|platformError}`，另附预算、
  duration及 top-level traceId/errorId；它不调用
  `response_error_to_telemetry_map`，不含 provider payload、source或stack。
- eval typed restricted sink在 provider heap存活时接收 provider owner、最终 correlation、
  typed source、request-local stack和有限 cause。Host只把该 closed value投影为
  `visibility=restricted`事件；local/remote-boundary frame保持结构化，path/function/open attrs
  没有输入 seam。
- sink失败不会改变 fixed结果：
  `export_provider_failure_with_diagnostic`先取得 fixed carrier，再 best-effort submit并忽略 submit
  error，最后返回同一个 carrier。Host emitter拒收只返回 capability error，不会替换 response。
- producer对 top-level correlation、attrs/error和 event总预算执行截断/敏感 key redaction；
  Telemetry store在 Mongo与内存 insert前再次执行 non-mutating、深度/字符串/数组/object预算及
  key/value redaction。
- `queryLogs`、`queryTrace`、`queryTraces`的共享底层 filter都固定
  `visibility=operational`；公开`/logs`、`/traces`、`/traces/:id`只调用这三个 surface。
  `queryRestrictedDiagnostics`只存在于 store interface/实现，没有 package export或 HTTP route，
  且强制 nonblank traceId或errorId、两者同时提供时取交集、最多1000条稳定排序。
- Mongo与 in-memory共同消费同一 operational/restricted filter builder。Mongo保留既有索引，
  并有`visibility_topic_ts_desc`、`visibility_trace_ts_asc`和
  `visibility_error_ts_asc`；Telemetry完整 suite同时覆盖共享 filter/index结构、内存行为、
  storage redaction和公开 route隔离。

### 4. C1 证据诚实性

结论：`PASS`。

- C1 scenario fixture只含 C0 case名、correlation、private sentinel、A/B/C owner/source/local-stack
  场景事实和 safe message；不含 envelope或`payloadUtf8`。Rust与Router均按 case名读取唯一 C0
  corpus bytes，Telemetry只读取同一 case expected correlation。
- C1 Rust target消费 production typed request carrier、`ResponseEvent`、transport encoder/decoder、
  reverse outbound与 Host telemetry adapter；Router target消费 production strict validator、
  dispatcher、fixed mapper及实际 Assembly HTTP/WS gateway；Telemetry target消费 production batch
  admission、store和真实 query server。测试没有另写 parallel classifier、wire codec或 query
  visibility策略。
- C1手工构造的三份`RestrictedServiceDiagnostic`只证明 production Host projection。它与本轮实际
  运行的真实 ordinary三跳 selector及顶层
  `service_error_channel_contract_operation_restricted_service_diagnostic_real_lanes`
  selector组合；没有把手工值表述为真实 eval生成或单进程 Rust→Node live链。
- Router gateway probe从 production strict view构造 fixed mapper error；同 target内另一个 probe
  证明 actual dispatcher只产生该 mapper且 exact-forward同一 header/payload对象。Telemetry使用
  production in-memory store与真实 HTTP routes；Mongo只由 production filter/index结构和同一完整
  suite验证，未宣称 Mongo live。

## Selector 枚举与抽查结果

执行前先枚举并确认非零：

```text
eval ordinary three-hop filter       1
eval ContractOperation real-lanes    1
Host P5-F345 target                  2
Router指定3文件                     19
Telemetry完整 suite                 19
```

枚举命令：

```bash
cargo test -p skiff-runtime-eval \
  restricted_service_diagnostic_ordinary_three_hop_preserves_bytes_and_local_stacks -- --list
cargo test -p skiff-runtime-eval \
  service_error_channel_contract_operation_restricted_service_diagnostic_real_lanes -- --list
cargo test -p skiff-runtime-host --test p5_f345_service_error_convergence -- --list
pnpm --filter @skiff/router exec vitest list \
  tests/service-error-cross-layer-convergence.test.ts \
  tests/assembly-http-gateway-stream.test.ts \
  tests/assembly-websocket-gateway.test.ts
pnpm --filter @skiff/telemetry exec vitest list
```

实际抽查：

```text
cargo test -p skiff-runtime-eval \
  restricted_service_diagnostic_ordinary_three_hop_preserves_bytes_and_local_stacks
  PASS: 1 passed

cargo test -p skiff-runtime-eval \
  service_error_channel_contract_operation_restricted_service_diagnostic_real_lanes
  PASS: 1 passed

cargo test -p skiff-runtime-host --test p5_f345_service_error_convergence
  PASS: 2 passed

pnpm --filter @skiff/router exec vitest run \
  tests/service-error-cross-layer-convergence.test.ts \
  tests/assembly-http-gateway-stream.test.ts \
  tests/assembly-websocket-gateway.test.ts
  PASS: 3 files / 19 tests

pnpm --filter @skiff/telemetry test
  PASS: 7 files / 19 tests
```

Rust命令只报告仓库既有 unused/dead-code warnings。未运行 workspace/root、stable/live、
Mongo live或依赖安装。

## 结构反搜与合法残余命中

执行了 F333 指定的三组反搜，并逐个检查 production调用方向：

```bash
rg -n \
  'RuntimeErrorPayload|ResponseError|UnhandledServiceError|runtimeErrorStatus|response_error_to_telemetry_map' \
  runtime/request-contract runtime/request runtime/transport runtime/host router telemetry

rg -n \
  'FixedServiceFailure|FixedServiceResponseFailure|OpaqueServiceError|ServiceErrorEnvelope' \
  runtime/request-contract runtime/request runtime/transport runtime/host router telemetry

rg -n \
  'sourceId|sourceFrame|sourceFrames|frames|stack|function|module_path|symbol_path|provider-private-secret' \
  runtime/transport router/src/router router/src/gateway
```

人工分类如下。

1. 第一组合法残余：
   - `ResponseError`/`RuntimeErrorPayload`仍服务 control、protocol、cancel、timeout、
     pre-ingress及其它 generic surface；
   - `response_error_to_telemetry_map`只有
     `request_supervisor::complete_error`和 control-plane route error两个 production caller；
     fixed assembly分支调用独立`complete_fixed_service_failure`；
   - `RuntimeResponseError`/`runtimeErrorStatus`用于 explicit control或本地 protocol/gateway错误；
     `RuntimeDispatcher::rejectRequest`只在 control branch构造它，fixed branch构造
     `FixedServiceResponseError`；
   - `UnhandledServiceError`的 production owner仍是 request-local user exception的 opaque generic
     fallback；fixed main path在此前由 typed extraction截获。其它命中为 tests。
   未发现 fixed依赖上述 generic值或 classifier的非法命中。
2. 第二组形成连续 typed链：
   eval `RuntimeError::FixedServiceFailure` →
   request typed extraction →
   request-contract `ResponseEvent::FixedServiceFailure` →
   transport strict v2 encode/decode →
   Router strict view/dispatcher/fixed mapper →
   Host router-session reverse `OutboundResponse::FixedServiceFailure`。
   未发现第二 envelope/codec或 fixed→generic flatten的 production接线。
3. 第三组原始`function`pattern的大量 Router命中只是 TypeScript function声明或 actor/request target
   文本；`module_path`/`symbol_path`只属于 runtime request metadata，不属于 response.error或
   gateway serializer；`frames`命中为 transport test locals或“typed binary runtime frames”
   protocol提示；`stack`唯一 fixed-wire命中是 C0 invalid corpus的额外字段负例。更窄的
   sentinel/source/stack反搜在 production external serializer为零。
4. 额外 multiline v1反搜只命中 C0 invalid case
   `legacy-v1-generic-response-error`。全局 v1常量仍合法服务其它 runtime frame；
   response.error生产者和decoder均使用独立 v2常量。

## 证据边界、失效条件与 verdict

本验收是 hermetic、跨 executable的组合证据，不声称运行单进程
eval→Rust Host→Node Router→Telemetry live链，也没有运行 Mongo live。真实 eval生成/import事实、
手工 Host projection、Rust/Node C0 bytes交接和 Telemetry store/query分别由其 production seam证明，
边界已显式区分。

Blocking issues：无。

若 exact candidate之后修改任务列出的 runtime production surface、`router/src`、
`telemetry/src`、C0 corpus/schema或 C1 scenario/test证据，本 verdict按 F346任务声明失效并需按影响面
重验。仅本 result提交本身不触发该失效条件。

Verdict：`PASS`。
