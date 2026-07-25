# P5-F345 Service error cross-layer convergence result

状态：`PASS`（C1 test-only 合流证据完成；无 production/public seam blocker；未修改 task
状态，未 push，未承接后续验收）。

## 候选与写入边界

- worktree：
  `/Users/geek/workspace/skiff-p5-f345-service-error-convergence`
- branch：`codex/p5-f345-service-error-convergence`
- task production 起点 commit：
  `335af586c132ffa74e04d5a58b515cf717c9d6ae`
- task production 起点 tree：
  `86e97cc0be2bb5e0891b0a27668464687c580187`
- 实际开发起始 HEAD：
  `808db67469eda3f464ddaaf6f882dbb55b8d3326`
- 实际开发起始 tree：
  `aaea985f5d71d39df9411b093301f692430cd7a5f`

`335af586..808db674`只新增本叶子 task 文档，production tree没有变化。

候选严格只新增任务列出的5个文件：

1. `testdata/package-service-contract-deployment/service-error-convergence.json`
2. `runtime/host/tests/p5_f345_service_error_convergence.rs`
3. `router/tests/service-error-cross-layer-convergence.test.ts`
4. `telemetry/tests/service-error-cross-layer-convergence.test.ts`
5. 本 result

production、既有 test/fixture/corpus、Cargo/package/lockfile、设计、父 task/result均没有修改。
验证期间只临时链接既有 Router/Telemetry依赖目录；交付前已删除。

## 同一场景事实与跨层连接

场景 fixture只拥有 task允许的事实：

- C0 corpus case引用：`internal-fixed-service-error`
- corpus相同的`trace-internal-1` / `error-internal-1`
- private sentinel：`provider-private-secret`
- A/B/C各自不同的 service、activation、operation、typed source与local stack期望
- external safe message：`Service request failed`

没有复制`ServiceErrorEnvelope`。Rust、Router均从
`runtime/transport/testdata/service-error-response-v2.json`按 case名字读取原
`payloadUtf8`；Telemetry读取同一 case的 expected correlation。

证据链按冻结 seam组合如下：

```text
既有真实 eval selectors
  ├─ ordinary three-hop：真实 export/import、逐跳 local/remote stack、exact fixed bytes
  └─ ContractOperation/ingress：真实顶层 lane与 typed restricted submit
             │
             │ 与 C1 手工值不互相替代
             ▼
C1 typed RestrictedServiceDiagnostic
  └─ production eval capability adapter
       └─ Host production projection：A/B/C 三条 restricted event

C0 internal payloadUtf8 exact bytes
  └─ typed eval/request carrier
       └─ ResponseEvent::FixedServiceFailure
            └─ Rust v2 frame / dedicated decode / outbound carrier × A/B/C
                 └─ Router C0 strict view
                      ├─ actual RuntimeDispatcher unaryFrame / fixed mapper
                      └─ actual Assembly HTTP / WebSocket gateway serializers

同一 correlation
  └─ operational event + A/B/C restricted events
       └─ Telemetry admission / store
            ├─ queryLogs/queryTrace/queryTraces + public HTTP routes：operational only
            └─ store-only restricted reader：exactly three, redacted
```

## Host / wire证据

新增 Rust target有2个 selector：

1. `c0_internal_bytes_cross_three_typed_host_wire_hops_and_control_stays_generic`
   - strict decode C0 Internal case；
   - 依次执行 typed eval/request extraction →
     `ResponseEvent::FixedServiceFailure` → Rust v2 encode →
     dedicated decode → outbound fixed carrier；
   - A/B/C三次 payload均与原 corpus UTF-8 bytes byte-for-byte相等；
   - correlation始终等于 fixture/C0；
   - matching code/message control仍为
     `ValidatedResponseErrorFrame::Control`与`OutboundResponse::Error`。
2. `production_eval_context_projects_three_correlated_restricted_hops_beside_one_safe_event`
   - operational事件只含有限`fixedService/internalError`、预算与 top-level correlation；
   - 三份 typed diagnostic通过
     `eval_capability_adapter::effects(...).telemetry_context()`进入 F340 production Host
     projection；
   - A/B/C owner/source/local stack各不相同、correlation相同；
   - B/C只有自己的 local frame加一条脱敏 remote-boundary frame，不继承下游 local frame；
   - operational不含 stack/source/sentinel，restricted不含 path/function/open payload。

同时运行：

- `restricted_service_diagnostic_ordinary_three_hop_preserves_bytes_and_local_stacks`
- `service_error_channel_contract_operation_restricted_service_diagnostic_real_lanes`

前者是实际 ordinary多跳执行，不是手工 diagnostic：它证明真实 export/import保持 exact bytes，
terminal/relay各自提交 provider-local diagnostic，caller/relay import各自只有 local调用点和
remote-boundary frame。后者证明 ordinary、async unary、stream与顶层 ingress/ContractOperation
共享真实 channel与 typed restricted lane。

本 C1 Host projection只证明“现有 production projection如何处理同场景三份 typed
diagnostic”；上述两个 eval selector才是 diagnostic由真实 eval产生的证据，两者明确组合，不把手工
值冒充 eval端到端执行。

## Router证据

新增 Router target有2个 selector：

1. C0 strict seam对同一 payload返回原`Uint8Array`对象、原 header对象和相同
   Internal/correlation view；actual `RuntimeDispatcher.dispatchBinaryFrame`返回同一 header/payload
   对象，没有重编码。ordinary pending只产生`FixedServiceResponseError`；matching control只产生
   `RuntimeResponseError`。
2. 同一个 strict view生成的 fixed mapper error实际进入
   `AssemblyHttpGateway`与`AssemblyWebSocketGateway`：
   - HTTP 500只返回`FixedServiceError`、fixture safe message及`traceId/errorId`；
   - WebSocket upgrade failure只返回相同 safe message及 correlation；
   - raw HTTP/WS bytes不含 sentinel、sourceId/sourceFrame/sourceFrames、frames、stack、
     function、path或encodedPayload。

任务指定的5文件 Router组合共38个 selector通过，其中既有 actual endpoint、dispatcher、
HTTP stream gateway与WebSocket gateway selector继续覆盖：

- endpoint只经 C0 strict v2 seam admission；
- fixed/control payload presence与 malformed fail-closed；
- fixed/control unaryFrame exact转发与互斥 mapper；
- HTTP unary/stream fixed redaction；
- WebSocket upgrade与receive close reason的相同 safe fact及123-byte UTF-8边界。

## Telemetry证据

新增 Telemetry target有1个 selector。它先通过现有 strict
`validateTelemetryBatch`，再向同一个 production in-memory store插入：

- 1条 operational log/error event；
- A/B/C 3条 restricted trace diagnostic；
- 4条均使用 fixture/C0相同`traceId/errorId`。

验证结果：

- store `queryLogs`、`queryTrace`、`queryTraces`均只返回1条 operational；
- 真实`/logs`、`/traces`、`/traces/:id`也均只返回该 operational event；
- top-level`errorId`可以精确关联，ordinary结果不含 stack/sentinel；
- `/restricted-diagnostics`为404，没有公开 restricted route；
- store-only `queryRestrictedDiagnostics({traceId,errorId})`恰好返回 A/B/C三条；
- 每条保留各 hop source、local stack和有限 remote-boundary结构；
- fixture private sentinel经 storage redaction成为`[REDACTED]`；
- 缺 correlation的 store-only读取继续 fail closed。

Telemetry完整 suite共7 files / 19 tests通过，包含既有 protocol、Mongo filter/index结构、
storage redaction与真实 public query route证据。

## W1 / S1 probe矩阵

| probe | 当前候选证据 | 结论 |
| --- | --- | --- |
| W1-P1 | F340 6项回归覆盖 public/Internal/platform typed request/host；C1用唯一 Internal场景走三跳 | PASS |
| W1-P2 | C0 transport 3项 + C1 Rust三次 v2 encode/dedicated decode exact bytes | PASS |
| W1-P3 | Router C0 protocol 2项 + C1同一 Internal case strict view与对象身份 | PASS |
| W1-P4 | C1 actual Router unaryFrame exact对象/bytes；C1 Rust reverse outbound carrier；F340 reverse session | PASS（组合证据） |
| W1-P5 | C0 matching control、C1 Rust/Router matching control与既有 dispatcher selector | PASS |
| W1-P6 | C1 actual Assembly HTTP/WS输出 + 既有 HTTP/WS actual selector | PASS |
| W1-P7 | F340 request trace应用/typed correlation + C1 corpus correlation operational event；actual gateway trace owner由既有 selector覆盖 | PASS（组合证据） |
| S1-P1 | 真实 ordinary three-hop selector + 真实 ContractOperation/ingress selector + C1 production Host三投影 | PASS（真实 eval与投影分证据） |
| S1-P2 | C1一条 operational/三条 restricted混存，三个普通 store/HTTP query与 store-only reader | PASS |

## 负向证据

| probe | 证据 | 结论 |
| --- | --- | --- |
| W1-N1 | C0 Rust/TS shared corpus；multiline反搜无`skiff-runtime-frame-v1` service `response.error` producer | PASS |
| W1-N2 | C0 strict corpus、actual endpoint selector及 payload-presence selector | PASS |
| W1-N3 | C0 Rust/TS strict shared corpus selector | PASS |
| W1-N4 | Rust每 hop比较原 bytes；Router strict seam与unaryFrame比较对象身份及 bytes；无 stringify/re-encode | PASS |
| W1-N5 | Rust raw frame、HTTP body、WS raw upgrade bytes和ordinary telemetry均反搜全部 forbidden字段/sentinel | PASS |
| W1-N6 | Host fixed分支调用`complete_fixed_service_failure`，generic helper只在`complete_error`；Router fixed/control显式 union，C1动态类型互斥 | PASS |
| S1-N1 | strict batch admission；ordinary filters显式`visibility=operational`；restricted reader强制 correlation；storage sentinel redaction | PASS |
| S1-N2 | real eval selector限定逐 hop export/import与 local stack；C1三次 exact bytes/correlation；三条 projection与store结果恰好各一 | PASS |

反搜中的合法命中：

- `response_error_to_telemetry_map`仍由 generic control
  `request_supervisor::complete_error`及 control-plane使用；fixed assembly分支调用独立
  `complete_fixed_service_failure`。
- `RuntimeResponseError`和`runtimeErrorStatus`仍用于 control/protocol/gateway error；
  `RuntimeDispatcher.rejectRequest`对`serviceError`显式构造
  `FixedServiceResponseError`，只有 control分支构造`RuntimeResponseError`。
- 全局`skiff-runtime-frame-v1`仍是 actor/request/control等其它 frame版本；
  `response.error`有独立`RESPONSE_ERROR_FRAME_SCHEMA_VERSION=v2`。multiline反搜没有 v1与
  `response.error`组成 producer。

## Selector枚举与验证

执行前枚举并确认非零：

- C1 Host：2
- 两个真实 eval filter：各1
- Router指定5文件：38（含 C1 2）
- Telemetry完整 suite：19（含 C1 1）

最终候选验证：

```text
cargo test -p skiff-runtime-eval
  restricted_service_diagnostic_ordinary_three_hop_preserves_bytes_and_local_stacks
  PASS: 1 passed

cargo test -p skiff-runtime-eval
  service_error_channel_contract_operation_restricted_service_diagnostic_real_lanes
  PASS: 1 passed

cargo test -p skiff-runtime-host --test p5_f345_service_error_convergence
  PASS: 2 passed

pnpm --filter @skiff/router exec vitest run <task指定5文件>
  PASS: 5 files / 38 tests

pnpm --filter @skiff/telemetry test
  PASS: 7 files / 19 tests

pnpm --filter @skiff/router run type-check
  PASS

pnpm --filter @skiff/telemetry run type-check
  PASS

cargo test -p skiff-runtime-host --test p5_f340_service_error_host
  PASS: 6 passed

cargo test -p skiff-runtime-transport service_error_response_v2
  PASS: 3 passed

pnpm --filter @skiff/router exec vitest run tests/protocol.test.ts
  -t 'response.error declarative oneOf|shared service_error_response_v2 corpus'
  PASS: 2 passed / 45 skipped

rustfmt --edition 2021 --check
  PASS（新增 Rust文件）

git diff --check
  PASS
```

Rust命令存在仓库既有 warning，没有新增编译错误。没有运行 workspace/root/stable/live或Mongo
live验证。

## 证据边界与 blocking

Blocking：无。现有 public test seam足以完成 C1，未增加 production/test-support API。

此 C1是 hermetic、跨 executable的组合证据，不宣称存在一个单进程
“eval → Rust Host → Node Router → Telemetry service” live harness：

1. 真实 eval selectors使用 eval crate内部真实 assembly fixture与 probe；允许写入范围不允许修改
   eval测试，也没有公开 seam把那次执行捕获的内部 diagnostic对象跨 test executable导出。
2. 因此 C1严格把“真实 eval产生/转发事实”与“同场景 typed diagnostic进入 production Host
   projection”组合；它没有用后者替代前者。
3. Rust/Node边界以唯一 C0 corpus的同一 UTF-8 payload bytes为交接事实；两个 consumer各自证明不
   重编码。
4. Telemetry使用 production in-memory store与真实 HTTP query server；Mongo live不在本 task
   验证范围，Mongo filter/index结构继续由完整 Telemetry suite覆盖。

这些是 test-only证据形态的明确边界，不是 implementation blocker。
