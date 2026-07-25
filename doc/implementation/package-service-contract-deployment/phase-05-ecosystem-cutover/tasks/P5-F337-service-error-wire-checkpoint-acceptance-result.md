# P5-F337 Service error wire checkpoint acceptance result

状态：`FAIL`。Blocking issues：1。

本结论只验收 F336 的 C0 shared response-error wire / telemetry checkpoint。由于 shared
Router declarative schema 尚未表达与 TypeScript interface、manual validator 和
header+payload seam 相同的判别 union，C0 不能冻结，H/R/T fan-out 不能解除。本结论不代表
W2-W、A6 或 Phase 5。

## Exact candidate 与只读边界

- 候选 commit：`fb29f806911b5dea3f1334e3d1af096248292897`
- 候选 tree：`eda84af5ba318f48c3a2b7b682b932c6855ce6c8`
- F336 implementation commit：
  `ba7ac5c5a9e79d5728120541a2839f17a4db690c`
- F336 implementation tree：
  `73eae4dc367b30c32b2262183169f16e63b4323d`
- F336 merge commit：
  `583a77ae2c108902075af62985e530922aefcc8c`
- 验收起始 HEAD：
  `923ba4fde2e7505046dfb200f3e5feb5e6bcbee6`
- 验收起始 HEAD tree：
  `922e5c4e09cd7e5546bce586ee34cdc9cd987fbe`
- worktree：
  `/Users/geek/workspace/skiff-p5-f337-error-wire-acceptance`
- branch：
  `codex/p5-f337-error-wire-acceptance`

`ba7ac5c5..fb29f806`只修改 F336 task 状态；`fb29f806..923ba4fd`只新增
F337 acceptance task。受验 shared production、tests 和 fixtures 在 F336 implementation
之后没有额外变化。

验收只读 production/tests/fixtures；未修改实现、测试、fixture、task 或设计，未 push。唯一写入是本
result。

## Blocking issue

### B1：Router declarative schema 没有表达 fixed/control 判别关系

TypeScript interface、manual validator 和 header+payload seam 已经表达 exact union：

- `router/src/protocol/envelope.ts:552-569`：
  `fixedService`没有`error`，`control`必须有`error`；
- `router/src/protocol/runtimeProtocol.ts:4120-4147`：
  manual validator按`errorKind`选择 exact allowed field set，并要求 control `error`；
- `router/src/protocol/runtimeProtocol.ts:2254-2310`：
  header+payload seam再分别要求 fixed 非空 payload、control 空 payload。

但 declarative schema 仍是一个 optional-property bag：

- `router/src/protocol/runtimeProtocol.ts:807-822`把`errorKind`和可选`error`放在同一个
  properties map；
- `router/src/protocol/runtimeProtocol.ts:1542-1549`只要求
  `schemaVersion/type/requestId/errorKind`，没有表达
  `fixedService => error forbidden`或`control => error required`。

因此按 declarative schema 本身：

1. `errorKind: "fixedService"`同时携带 generic `error`仍在声明字段集合内；
2. `errorKind: "control"`缺少`error`也不违反 required 集合。

这两种形状分别正是 shared corpus 的`fixed-carries-generic-error`和
`control-missing-error`负例。Rust decoder和 TS manual seam会拒绝它们，但
`router/tests/protocol.test.ts:184-249`只把 corpus送入 manual seam；对 declarative schema
只断言 nested `error.additionalProperties === false`，没有证明 variant presence/absence parity。

这违反 F337 必须判断项 5，也保留了 F333 要消除的第四份 wire 描述漂移。即使当前 runtime admission
主要调用 manual validator，高风险 shared checkpoint 也不能向 H/R/T 发布两份含义不同的 schema。
需要 shared owner修正 declarative schema/其表达能力并用同一 corpus覆盖该表示后重新验收。

## 独立验收矩阵

| 验收面 | 结论 | 独立证据 |
| --- | --- | --- |
| fixed carrier唯一 owner | PASS | `FixedServiceResponseFailure`只有`runtime/capability-context/src/response.rs:36`一个定义；F336对 capability-context 只做 crate-root additive re-export。`runtime/request-contract/src/response_event.rs:29-33`直接携带同一类型，没有第二 envelope/carrier，也没有 capability 语义变化。 |
| v1/v2 hard cut | PASS | binary magic/version及普通 runtime frame仍为 v1；Rust/TS只有`response.error`使用`skiff-runtime-frame-v2`。shared transport/protocol owner没有 v1 response-error producer、reader、dual path或 fallback；旧 consumer命中仅位于待迁移 H/R。 |
| Rust exact union | PASS | `ResponseErrorFrameHeader`为 serde internally tagged、deny-unknown-fields union；dedicated validation检查 exact version/type、非空 requestId、payload presence、control非空 code/message和 400–599 status。fixed/control只按 enum variant分流，generic control不会按 code/message/shape升级。 |
| canonical opaque bytes | PASS | transport只调用`OpaqueServiceError::decode`；fixed producer取`into_encoded_bytes()`，decoder把收到的原 Vec 保存在 opaque owner，outbound mapper直接交回 typed carrier。public/Internal/platform三种 mapper round trip均 byte-equal。TS seam返回原`Uint8Array`对象与只读 view，不 stringify 或重编码。 |
| TS 四层 union parity | **FAIL** | interface、manual validator、header+payload seam正确；declarative schema没有表达 error presence与 discriminator 的条件关系。见 B1。nested `additionalProperties:false`和 strict envelope view本身正确。 |
| 4正/30负 shared corpus | PASS | `service-error-response-v2.json`精确含 public/Internal/platform/control 4正和30负；Rust protocol与Router TS直接读取同一文件。负例覆盖 v1、kind/version/type/requestId、variant混合、extra/missing、payload presence、malformed envelope、identity/correlation空白和 control约束；没有另一份等价 wire fixture。B1 是 corpus harness 没覆盖 declarative 表示，不是 corpus内容缺失。 |
| telemetry parity | PASS | Rust transport、Router TS、telemetry TS均要求`visibility=operational|restricted`、可选但存在时非空 top-level `errorId`，且 restricted必须同时有非空 traceId/errorId；unknown visibility/field失败。共同 observability fixture提供4正与相关负例，协议常量保持`skiff-telemetry-v1`。 |
| C0 scope 与 consumer可接线性 | PASS（scope）/ blocked（fan-out） | diff没有越界实现 host projection、gateway policy或 telemetry storage/query/redaction。typed frame seam、opaque bytes、finite view和 telemetry fields足以让 H/R/T不新增 compatibility或第二 schema；但 B1 修复前不能正式解除 fan-out。 |
| Cargo dependency方向 | PASS | transport只新增对更低层`skiff-runtime-model`的直接依赖；dependency tree为 transport→model及 transport→request-contract→capability-context→model，无回边。`Cargo.lock`只在 transport package dependency list新增这一项。 |

## 独立 selector 与执行结果

先列出 selector：

```text
service_error_response_v2_mapper_                                      2
telemetry_shared_fixture_requires_visibility_and_restricted_correlation 1
```

随后用 list 返回的完整 canonical test path运行：

```text
response_mapper::tests::service_error_response_v2_mapper_round_trip_preserves_fixed_payload_bytes
  1/1 PASS

response_mapper::tests::service_error_response_v2_mapper_keeps_matching_generic_control_untyped
  1/1 PASS

protocol::tests::telemetry_shared_fixture_requires_visibility_and_restricted_correlation
  1/1 PASS
```

第一条证明 public/Internal/platform三种 fixed mapper encode→strict decode→outbound 保留 exact
bytes；第二条证明与 Internal 相同 code/message 的 generic control仍为
`OutboundResponse::Error`；第三条同时抽查 operational/restricted正例和 visibility、correlation、
unknown-field负例。

一次把短 selector与`--exact`组合的尝试得到0 tests，不计入证据；上述三次均改用 list 返回的完整 path
并各执行1个非零测试。

按任务约束未重复运行完整 shared-corpus合流探针，也未运行完整 Router、telemetry、eval、workspace、
root、stable或 live。另执行`cargo tree`确认依赖方向，`git diff --check`通过。

## 反搜结论

1. production中`FixedServiceResponseFailure`只有一个定义；request-contract只 re-export/复用。
2. shared transport/Router protocol中的 response-error writer/strict reader只生成或接受 v2；
   `RUNTIME_FRAME_SCHEMA_VERSION`仍用于其它 frame。v1/errorKind缺失形状只出现在 shared负例或下游待迁移
   consumer。
3. shared fixed Rust decode只命中`OpaqueServiceError::decode`；TS只生成 finite view并保留原 bytes。
4. shared owner没有按 Internal code/message、status或 details形状恢复 fixed。
5. telemetry三镜像的新增字段、allowed-field集合和 restricted correlation规则一致。

## Consumer 断点判断

以下均与当前 production真实相符，是 F336 预期留下的 hard cut；它们不是 B1 的替代修复。

### H：request / host / session

- `runtime/request/src/error.rs:62-69,127`仍把
  `Eval(FixedServiceFailure)`经 generic `WirePayload`压平；
  `runtime/host/src/host/request_entry/assembly.rs:160-184`仍只生成
  `ResponseEvent::Error`。
- `runtime/host/src/host/http_response_ceiling.rs:15-22`的 exhaustive match没有
  `ResponseEvent::FixedServiceFailure`分支。
- `runtime/host/src/host/router_session.rs:550-559`仍使用 generic typed-header decode、强制 payload为空、
  直接访问旧 struct field并调用旧单参数 mapper；必须切到 dedicated v2 decode并把 fixed交回 typed
  outbound carrier。
- `runtime/host/src/telemetry.rs:90-121`和相邻 host telemetry构造器尚未提供新增的
  visibility/errorId，typed restricted diagnostic sink也尚未投影。

### R：Router dispatcher / gateway

- `router/src/router/runtimeEndpoint.ts:600-613`仍发送 v1、无 errorKind 的 generic
  response-error；`644-651`仍强制全部 error payload为空并直接读取`header.error`。
- `router/src/router/runtimeDispatcher.ts:647-681`仍把 union假定为 generic error、为 unaryFrame重建
  v1 header+空 payload，并把普通 pending交给`RuntimeResponseError`按 code/status分类。
- production HTTP/WebSocket gateway仍没有 explicit fixed mapper、safe correlation与脱敏策略。

### T：telemetry admission / storage / query

- protocol admission现在会接受合法 restricted event，但
  `telemetry/src/mongoStore.ts:135-159,215-233`和
  `telemetry/src/queryApi.ts:71-99`尚未按 visibility fail-closed过滤；当前普通 logs/traces查询会读到
  restricted记录。
- store/redaction/query尚未实现 restricted access split、errorId索引/查询和对应防御性测试。
- Router/host现有 producer literal仍缺必填 visibility；例如
  `router/src/router/httpGateway.ts:1007-1035`。

shared seam本身已经提供 H/R/T所需 typed carrier、original payload、finite view及 telemetry字段；消费者
不需要新增 v1 adapter或第二 schema。但在 B1 修正并重验前，H/R/T fan-out仍保持 blocked。

## Blocking、non-blocking 与 verdict

Blocking：

1. Router declarative response-error schema不表达 fixed/control exact union，且 shared corpus harness
   没有覆盖这一表示。

Non-blocking：无。上列 H/R/T 项是预期 consumer断点，不是 C0 non-blocking尾项。

Verdict：`FAIL`。不得冻结 C0或解除 H/R/T；修正 B1 后只需重验 shared TS schema/validator/corpus parity
及本验收直接受影响面，不应连带推翻 fixed carrier、Rust exact-byte或 telemetry字段证据。
