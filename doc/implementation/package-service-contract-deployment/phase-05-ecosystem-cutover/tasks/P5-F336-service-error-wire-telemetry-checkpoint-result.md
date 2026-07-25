# P5-F336 Service error wire and telemetry checkpoint result

状态：`PASS`（C0 shared checkpoint 实现完成；未 push，未承接 H/R/T、combined probe 或独立验收）。

## 候选与边界

- worktree：`/Users/geek/workspace/skiff-p5-f336-error-wire-checkpoint`
- branch：`codex/p5-f336-error-wire-checkpoint`
- 实现起点：`6173966e38a65ec34745427039d98e6f9ac66d5f`
- 起点 tree：`117d60c00903062082fbe64340a9da0492ed896e`
- `6173966e`只对本任务补充了一个精确边界：允许
  `runtime/capability-context/src/lib.rs` additive re-export既有
  `FixedServiceResponseFailure`。本实现没有修改该类型、`response.rs`或F334 diagnostic seam。
- production diff只位于任务允许的 capability-context re-export、request-contract、transport、
  Router protocol和telemetry protocol owner；fixture/test diff也只位于任务列出的 shared corpus与
  protocol tests。没有修改 model/eval/request/host、Router consumer/gateway、telemetry
  server/store/query/redaction、compiler/std或权威设计正文。

## 实现结果

1. capability-context crate root公开既有`FixedServiceResponseFailure`；request-contract的
   `ResponseEvent::FixedServiceFailure`直接携带并 re-export同一类型，没有新建 envelope/carrier。
2. Rust transport新增仅供`response.error`使用的
   `RESPONSE_ERROR_FRAME_SCHEMA_VERSION = "skiff-runtime-frame-v2"`。binary magic/version及其它 frame
   继续使用 v1。`ResponseErrorFrameHeader`成为`fixedService | control`严格判别 union。
3. Rust encoder、strict decoder和 mapper统一检查 exact version/type/kind、非空 requestId、variant
   exact字段、payload presence、control非空 code/message及400–599 status。fixed只调用
   `OpaqueServiceError::decode`，并把收到的原始 bytes保存在既有 opaque carrier中；三种 fixed
   mapper round trip均 byte-equal。
4. Router TS interface、declarative schema和manual validator镜像同一 v2 union；nested
   `error.additionalProperties`统一为`false`。`validateResponseErrorFrame(header, payloadBytes)`同时接收
   header与`Uint8Array`，strict解析只读 envelope view并原样返回同一个 payload对象，不重编码或
   stringify。
5. TS fixed view只接受三种 canonical envelope，严格检查 exact字段、有限 platform identity、
   owner/key/type/correlation无空值或外围空白、非空 byte array及Internal nested payload exact字段。
6. Rust、Router TS及telemetry service TS一次冻结 telemetry parity：必填
   `visibility: operational | restricted`、可选且存在时非空的 top-level`errorId`；restricted必须同时有
   非空`traceId/errorId`。Rust raw DTO以`deny_unknown_fields`解码；两份 TS validator接受同一字段和值集合。
7. telemetry protocol常量保持`skiff-telemetry-v1`。没有增加 stack顶层字段；restricted内容仍只放既有
   `error`对象，storage/query/redaction由T继续实现。

## Rust / TS / corpus / telemetry parity矩阵

| 验收面 | 结果 | 代码与探针证据 |
| --- | --- | --- |
| typed carrier | PASS | capability-context只 additive re-export；request-contract enum直接使用同一`FixedServiceResponseFailure` |
| Rust v2 union | PASS | fixed header无`error`、control header必有`error`；serde拒绝unknown/missing/variant混合字段，显式 validator再冻结version/type/request/payload/status |
| Rust exact bytes | PASS | public、Internal、platform分别通过`ResponseEvent` mapper encode→strict decode→outbound mapper；`OpaqueServiceError::encoded_bytes()`与fixture UTF-8 bytes逐字节相等 |
| TS strict seam | PASS | shared 4个正例均得到相同 kind/identity/traceId/errorId view；返回的`payloadBytes`与输入以引用相等证明未复制/重编码 |
| TS exact envelope | PASS | shared mutations覆盖unknown kind/platform、missing/extra字段、owner/key/type/correlation空白、empty/non-byte encoded payload及Internal nested extra |
| control不升级 | PASS | code=`InternalError`且message与Internal固定文案相同的合法control仍返回control；Rust outbound仍是generic`OutboundResponse::Error` |
| shared corpus | PASS | `service-error-response-v2.json`含4个正例（public/Internal/platform/control）和30个负例；Rust与Router读取同一文件 |
| telemetry parity | PASS | shared observability fixture含3个显式operational事件、1个带traceId/errorId的restricted事件和8个batch负例；Rust/Router/telemetry三侧消费同一事实 |
| telemetry fail closed | PASS | missing/unknown visibility、restricted缺trace或error、unknown top-level字段、operational空errorId均拒绝 |

shared corpus负例还覆盖：v1 generic、missing/unknown errorKind、header extra/missing字段、fixed带generic
error、control缺error、fixed空payload、control非空payload、malformed JSON、control空code/message及非法
status。没有 v1 reader/writer、dual path、fallback或按 code/message/shape升级 fixed。

## 临时 consumer断点

这些断点是任务明确留给 H/R/T 的 hard cut，不是 C0 blocker；本任务没有为保持全仓 type-check增加兼容。

### H：request / host / session

- `runtime/host/src/host/router_session.rs:551-559`仍用generic typed-header decode、强制空payload、
  直接字段`header.request_id`及旧单参数`response_error_to_outbound`；H须改用v2 strict decode并把
  fixed映射到既有typed outbound carrier。
- `runtime/host/src/host/http_response_ceiling.rs:15-22`以及相邻 request/host exhaustive matches尚未加入
  `ResponseEvent::FixedServiceFailure`分支；assembly entry仍生成generic`ResponseEvent::Error`。
- `runtime/host/src/telemetry.rs:94-124`构造`TelemetryEvent`时尚未提供必填visibility/errorId；
  restricted diagnostic sink仍由H接线。
- host router-session tests仍使用旧`ResponseErrorFrameHeader` struct literal和空payload预期。

### R：Router dispatcher / gateway

- `router/src/router/runtimeEndpoint.ts:605-614`仍生成v1、无errorKind的generic
  `response.error`；`644-650`仍强制所有error payload为空并直接读取`header.error`。
- `router/src/router/runtimeDispatcher.ts:649-681`仍把header收窄成generic error、重建v1 header+空payload，
  并交`RuntimeResponseError`分类；R须按explicit fixed/control分流并原样转发header+bytes。
- HTTP/WebSocket external fixed mapper、redaction和correlation policy仍未接入；本任务未触碰这些 consumer。

### T：telemetry admission / storage / query

- telemetry server/store/query/redaction未修改；restricted默认不可见和显式受限读取仍由T实现。
- `router/src/router/httpGateway.ts:1007`等现有 Router telemetry producer literal尚未补必填visibility；
  T/R应按其既定 ownership迁移 producer，不能在 shared protocol恢复 optional/default。
- telemetry非protocol测试中的手写`TelemetryEvent` fixture仍需在T迁移时补visibility；本任务只运行并拥有
  protocol parity test。

## 验证

以下均在最终代码状态运行：

```text
cargo test -p skiff-runtime-request-contract --lib --no-fail-fast
  3 passed

cargo test -p skiff-runtime-transport --lib service_error_response_v2 -- --list
  3 tests, 0 benchmarks

cargo test -p skiff-runtime-transport --lib service_error_response_v2 --no-fail-fast
  3 passed

cargo test -p skiff-runtime-transport --lib telemetry --no-fail-fast
  2 passed

cargo check -p skiff-runtime-transport --lib
  PASS

pnpm --filter @skiff/router exec vitest run tests/protocol.test.ts
  1 file, 46 tests passed

pnpm --filter @skiff/telemetry exec vitest run tests/protocol.test.ts
  1 file, 4 tests passed

git diff --check
  PASS
```

selector为非零。按任务约束未运行完整 Router/telemetry、eval、workspace/root、stable或live，也未运行
下游 consumer type-check；上列断点由逐点 production反搜确认。

## 反搜与结论

- `RUNTIME_FRAME_SCHEMA_VERSION`仍为v1；v2常量只用于`response.error` Rust/TS header、schema和decoder。
- production fixed encoder没有 generic error字段；control encoder没有payload。shared invalid corpus证明两者
  不能混合。
- fixed decode只经`OpaqueServiceError::decode`/TS strict view；mapper与TS seam均保留原payload bytes。
- generic `RuntimeErrorFramePayload`、`ResponseError`和Router control classifier仍保留给control surface；
  没有按值推断fixed。
- telemetry三份镜像均只有同一新增`visibility/errorId` shape；协议版本未变，未接storage/query行为。
- Blocking issues：无。C0可解除H/R/T；不得将本结果表述为W2-W、A6或Phase 5 PASS。
