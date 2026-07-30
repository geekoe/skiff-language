# P5-F340 Service error request/host/session consumer result

状态：`PASS`（H consumer 实现完成；完整 host lib-test 被本阶段尚未合流的 test-only
consumer 断点阻塞，已按任务例外给出精确证据并运行非零 focused selectors；未修改 task 状态，未
push，未承接后续验收）。

## 候选与写入边界

- worktree：`/Users/geek/workspace/skiff-p5-f340-service-error-host`
- branch：`codex/p5-f340-service-error-host`
- task production 起点 commit：
  `e3095ec642d49b59955f5f48a2950eafc9d92571`
- task production 起点 tree：
  `6b7fce6db07d7fde3b88609539150c53f5608e62`
- 实际开发起始 HEAD：
  `5fa0389e151a27f9cbd2906a7394658190e42491`
- 实际开发起始 tree：
  `b344fea27ed6da25ce899713d9b2397651edc93d`

生产写入均在任务允许文件内，只有一个已由任务预留的相邻 production 例外：
`runtime/host/src/error.rs`。原因是冻结后的 model 已移除 `TypeIdentity`，而 host production
仍引用该旧类型，导致本任务起点上的 `cargo check -p skiff-runtime-host` 无法编译。该文件只把
既有 catch projection 收敛到 canonical `CatchIdentity` /
`PlatformBuiltinErrorIdentity`；没有添加 compatibility alias、修改 error payload，或扩展到其它
crate。未修改 model service-error、capability-context crate、eval assembly execution、
request-contract、transport、Router、telemetry service、shared corpus、lockfile、权威设计或父
task/result。

## Typed response 路径与 exact bytes

1. `EvalRuntimeError::fixed_service_failure()`只递归穿过 typed diagnostic/source wrapper，并且只
   接受 `FixedServiceFailure(OpaqueServiceError)`。request 暴露严格 typed extraction 以及
   `FixedServiceResponseFailure` carrier；没有调用 `WirePayload::payload()`，也不按
   code/message/details 分类。
2. assembly 在任何 payload flatten 或 `RequestError::response_error()`之前先查询 typed carrier：
   fixed 进入 `ResponseEvent::FixedServiceFailure`及独立 supervisor completion；普通 control、
   cancel和其它 request error继续进入 `ResponseEvent::Error(ResponseError)`。相同
   code/message 的 generic control 测试确认不会升级。
3. fixed operational completion只投影有限
   `{kind: fixedService, causeKind: publicTypedError|internalError|platformError}`、duration和预算；
   top-level traceId/errorId直接来自 fixed envelope。production 搜索确认 fixed 分支没有调用
   `response_error_to_telemetry_map`。
4. Rust frame沿 shared `response_event_into_frame`写入原
   `OpaqueServiceError.encoded_bytes()`。session 的 `response.error`分支只调用 C0 dedicated
   `decode_response_error_frame`：fixed 非空 payload恢复 typed carrier，control保持空 payload；
   随后把验证后的原 bytes传给`response_error_to_outbound(header, payload)`，不 stringify、
   reserialize或按 generic DTO重建。
5. focused fixture分别用 public typed、internal、platform三种带不同 whitespace/布局的原始 bytes，
   逐跳断言 request carrier、Rust frame decode及 reverse outbound carrier byte-for-byte相等。
   fixed-empty与control-nonempty均 fail closed。HTTP ceiling production分支把 fixed视为合法 terminal，
   不把它计作 response body。

## Trace 与 telemetry 投影

- assembly `RequestTelemetryContext`在启动 supervisor前应用
  `RequestTraceFields::from_request(request)`，复用 Router ingress的 trace/span/parentSpan；普通
  telemetry显式由`telemetry_event`初始化为`visibility=operational`和`errorId=None`。fixed
  completion只覆写 envelope traceId/errorId，span/parentSpan仍来自同一 ingress context。
- production eval capability context用同一个 clone-safe
  `RuntimeTelemetryCapabilityContext`安装 F335 typed restricted sink；default discard不再是 assembly
  production路径。每次 typed submit只向同一个 request emitter发送一条
  `visibility=restricted`事件。
- restricted owner使用 diagnostic自带 provider service/activation/operation/request generation；
  correlation使用 diagnostic自带 traceId/errorId；request/runtime/span/parentSpan来自同一 request
  emitter。caller build没有被误标成 provider build。
- restricted error是闭合集合：有限 cause kind、typed source span或有限 synthetic reason，以及
  local typed site / remote-boundary service-operation-errorId frame。它不接受 display、payload、
  heap/runtime value、type address、源码路径、函数名或 open attrs。
- host producer redaction新增 top-level errorId截断；secret-key redaction覆盖 restricted error。
  event超预算时 attrs与error同时替换为有界 truncation marker并移除 message，因此完整 restricted
  stack也受 producer event budget约束。
- sink拒收返回 capability error；F335 eval consumer忽略该私有诊断提交失败，fixed carrier不被替换。
  focused测试同时断言拒收后 emitter为空且原 fixed encoded bytes未改变。

## Selector、测试与反向证据

先枚举并确认 request selector为非零 2 项：

```text
error::tests::fixed_service_failure_is_extracted_only_from_the_typed_eval_carrier
error::tests::fixed_service_response_failure_preserves_all_envelope_bytes
```

再枚举并确认 host focused target为非零 6 项：

```text
matching_generic_control_stays_generic_and_payload_rules_fail_closed
operational_fixed_event_uses_top_level_correlation_and_safe_error_shape
production_eval_context_projects_one_typed_restricted_event_to_the_same_emitter
request_to_wire_and_reverse_session_seam_preserve_three_fixed_payloads
restricted_projection_is_covered_by_secret_redaction_and_event_budget
restricted_sink_failure_does_not_mutate_fixed_bytes
```

最终候选验证：

```text
cargo test -p skiff-runtime-request
  PASS: 26 passed, 0 failed

cargo test -p skiff-runtime-host --test p5_f340_service_error_host --no-fail-fast
  PASS: 6 passed, 0 failed

cargo check -p skiff-runtime-host
  PASS

git diff --check
  PASS
```

co-located tests还覆盖 ingress trace字段应用、HTTP ceiling fixed terminal及 session fixed bytes；这些
lib-test在编译完整 host test harness前即被下述非本任务 fixture断点阻塞。production仍由
`cargo check`编译，相关公开 seams由上述 integration target执行。

反向搜索结论：

- `runtime/request/src`及`runtime/host/src`中没有 fixed carrier与
  `response_error_to_telemetry_map`或`WirePayload::payload()`组合；
- session不再以 generic `decode_typed_binary_frame::<ResponseErrorFrameHeader>`处理
  `response.error`，也没有遗留“所有 response.error payload必须为空”的判断；
- production restricted sink只有 assembly eval capability真实安装点，普通 telemetry构造默认
  operational；
- diff没有命中任务明确禁止路径。

## 完整 host suite 例外与 blocking

`cargo test -p skiff-runtime-host`在编译`skiff-runtime-host (lib test)`时以44个既有
test-only consumer错误退出；production host library已经由`cargo check`通过。错误位于尚未迁移到
当前冻结 model/artifact API的旧 fixtures，例如：

```text
runtime/host/src/loader/assembly_admission/tests/execution/artifacts.rs
  BoundaryOperationContract.errors / BoundaryErrorContract 已移除
  CallIr / StmtIr::Throw 缺少必需 site
  TypeDeclIr.discriminator 已移除
  PackageCallableSignature.throw_types 已移除

runtime/host/src/error/tests.rs
  仍使用已移除的 TypeIdentity、
  UserException::from_typed_payload / from_envelope

runtime/host/src/loader/assembly_admission/tests/execution/async_stream_cancel.rs
  仍使用已移除的 UserException::error_payload
```

这些断点在本任务 production域之外，且与本次 fixed host consumer focused target无重叠，因此没有
扩张为 artifact/loader/eval test consumer迁移。Blocking：本 implementation无 blocker；完整 host
lib-test仍需对应 consumer owner合流后由后续 gate owner复跑。本任务没有运行 workspace/root、
stable或 live验证。
