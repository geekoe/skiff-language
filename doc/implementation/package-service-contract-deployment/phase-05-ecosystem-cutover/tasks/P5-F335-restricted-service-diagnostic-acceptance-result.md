# P5-F335 Restricted service diagnostic acceptance result

状态：`PASS`。Blocking issues：无。

本结论只冻结 F334 的 RΔ restricted diagnostic handoff，并解除 F336 shared
wire/telemetry checkpoint；不代表 W2-W、A6 或 Phase 5 通过。

## Exact candidate 与只读边界

- production candidate：
  `a4bd73be4fa59dd20937aabd9ccd6519cda1d138`
- candidate tree：
  `fcac3e734101e7f085167b21a19f03e54f6a5639`
- F334 implementation commit：
  `03f29b05bc0a58f1b5d48f8b3c43bd0a7f69d5f0`
- 验收起始 HEAD：
  `72359c1c9e67c1b08141c890c6caaa0f7e813536`
- 验收起始 HEAD tree：
  `cfdc7ed6ae90845b2a41e65a57f0bab9511718f2`
- worktree：
  `/Users/geek/workspace/skiff-p5-f335-restricted-diagnostic-acceptance`
- branch：
  `codex/p5-f335-restricted-diagnostic-acceptance`

`03f29b05`是 candidate 的祖先。`03f29b05..a4bd73be`只有 F334 task
状态更新，`runtime/capability-context`与`runtime/eval`均无 diff；
`a4bd73be..72359c1c`只有新增 F335 task，受验 production/tests 无额外变化。

验收只读 production/tests；未修改实现、fixture、设计或 task 状态，未
push。唯一写入是本 result。

## 独立验收矩阵

| 验收面 | 结论 | 独立证据 |
| --- | --- | --- |
| typed value 与受限字段 | PASS | `runtime/capability-context/src/telemetry.rs`中的 value 只有显式 provider service/operation/activation/request-generation owner、最终 `ErrorCorrelation`、typed `InstructionSourceSite`、`ExceptionStackFrame[]`和三值 cause kind；无 serde derive、open attrs、payload/display、heap handle、`RuntimeValue`或`TypeAddr`。`serde_json::Value`仍只属于原有用户 `emit_native` surface。 |
| sink 与 `emit_native` 分离 | PASS | `RestrictedServiceDiagnosticSink::submit`是独立 trait/`Arc`，`TelemetryCapabilityContext::emit_native`未改；context clone共享同一 typed sink。默认 discard只由 constructor安装，eval失败分支仍真实调用 production submit seam，没有第二个绕过 submit 的 production export 路径。 |
| lane、时序与次数 | PASS | ordinary 在 fresh provider heap仍存活时调用 wrapper；async unary和server-stream共用 `async_stream_cancel.rs::export_provider_failure`，且都在 provider heap owner drop前调用；`ContractOperation` test effect在 `throw.setup_heap`仍存活时调用同一 wrapper。success、unary cancel、stream success/consumer cancel/request cancel/control和 `PackageCallable`均不进入 wrapper；每个失败分支只有一个调用点。 |
| additive R0 wrapper | PASS | `export_provider_failure_with_diagnostic`先调用冻结的 `export_provider_failure`取得最终 `OpaqueServiceError`，再只从最终 envelope复制 trace/error id并按 envelope enum映射有限 cause kind；submit返回值被 best-effort忽略，函数返回原 fixed carrier。没有按 message/code/status/shape分类，也没有修改 R0 API或分类实现。 |
| imported fixed/Internal 与逐跳栈 | PASS | R0对直接 `FixedServiceFailure`或 `RequestException`中的 imported fixed cause直接 clone原 `OpaqueServiceError`，不调用 allocator或codec。wrapper随后从当前 provider的 `RequestException`读取 source/stack，所以 relay B记录 B 的 local site和脱敏 C remote-boundary，而不读取 C 的本地栈；raw bytes和 correlation不变。独立阅读 ordinary三跳 fixture确认两次 import bytes相等、两份 provider diagnostic owner/stack分离。 |
| private/nonclosed/encode failure 隐私 | PASS | diagnostic从不持有 local carrier或错误值；private/nonclosed/encode failure在冻结 R0先收敛为 fixed Internal，sidecar只看到最终 correlation和 `InternalError` kind。generic diagnostic frame只被窄化解析为 typed `SourceSpanRef`；path、function、message和其它 JSON frame字段被丢弃。failing-sink负例同时证明 private sentinel不在 safe diagnostic且 fixed bytes byte-equal。 |
| 无第二 owner/codec/DTO | PASS | ordinary旧 `OrdinaryProviderFailureRecord`、spy与 probe symbols反搜为零；test recording sink只观察同一 production typed value。新增 production没有第二 error classifier、envelope codec、telemetry DTO或 response surface；model/request/host/transport/Router/telemetry/compiler/std 均无 production diff。 |
| F336/H 可接线性 | PASS | typed sink由 clone-safe capability context携带，owner、correlation、source、stack和有限 kind足以让 H 投影受限 telemetry；不需要读取 provider heap、generic JSON或 external response。当前 host仍使用 constructor的默认 discard，这正是 F336/H 边界内残余工作，不是 F334 blocker。 |

## Production 调用链与反搜

独立逐段读取：

- `runtime/capability-context/src/{telemetry.rs,lib.rs}`
- `runtime/eval/src/assembly_execution/{service_error_channel.rs,ordinary.rs,async_stream_cancel.rs}`
- `runtime/eval/src/{eval_context.rs,program_execution.rs,test_effect_registry.rs,capabilities.rs}`
- 对应 co-located restricted diagnostic、ordinary、stream和service-effect tests

反搜结论：

1. `export_provider_failure_with_diagnostic`的 production 结构只有一个定义和三个
   lane入口：ordinary、async/stream共享 helper、`ContractOperation` materializer。
2. `submit_restricted_service_diagnostic`的 eval production调用只有 wrapper内一处；
   其它命中是 capability method或 test。
3. `export_provider_failure`在 lane production不再直接调用；wrapper之外的命中均为
   R0定义或 test-only probe。
4. `OrdinaryProviderFailureRecord`、`record_ordinary_provider_failure`及旧 ordinary
   spy/probe反搜为零。
5. restricted value没有 `Serialize`/`Deserialize`实现或 serde annotation。
   `diagnostic_instruction_stack`只把既有 diagnostic frame中的 exact span窄化为
   `SourceSpanRef`，不保留 generic JSON。
6. changed lane没有新增 `ServiceErrorEnvelope` codec、canonical JSON编码或
   telemetry/response record。async文件中既有 `runtime_to_wire`仍服务普通 value/stream
   materialization，不承担错误分类或 fixed bytes重编码。

## 独立 selector 与执行结果

先列 selector并确认非零：

```text
restricted_service_diagnostic_server_stream_                                      2
service_error_channel_contract_operation_restricted_service_diagnostic_effect_throw 1
restricted_service_diagnostic_package_callable_typed_throw_submits_zero             1
restricted_service_diagnostic_private_sink_failure_preserves_fixed_bytes_and_safe_fields 1
```

实际只运行上述最小抽查：

```text
server-stream failure / request-cancel pair                         2/2 PASS
ContractOperation linked effect throw                               1/1 PASS
PackageCallable typed throw emits zero                              1/1 PASS
failing sink preserves exact fixed bytes and safe fields            1/1 PASS

合计                                                                 5/5 PASS
```

对应命令：

```bash
cargo test -p skiff-runtime-eval --lib restricted_service_diagnostic_server_stream_ -- --nocapture
cargo test -p skiff-runtime-eval --lib service_error_channel_contract_operation_restricted_service_diagnostic_effect_throw -- --nocapture
cargo test -p skiff-runtime-eval --lib restricted_service_diagnostic_package_callable_typed_throw_submits_zero -- --nocapture
cargo test -p skiff-runtime-eval --lib restricted_service_diagnostic_private_sink_failure_preserves_fixed_bytes_and_safe_fields -- --nocapture
```

按任务要求未重复运行已经提供 1/1 PASS 的 ordinary三跳合流探针；本次独立读取了
该 fixture及其 raw-bytes、per-hop owner/source/stack断言。测试只报告既有
`skiff-compiler-source` unused与 `skiff-runtime-linker` dead-code warnings，没有新增失败。

未运行完整 eval、workspace/root gate、stable或 live。

## Blocking、non-blocking 与残余风险

Blocking issues：无。

Non-blocking：

- F336/H尚未把默认 sink接到 host restricted telemetry。production现在会调用 typed
  seam，但默认实现有意 discard；接线必须保持它与用户 `emit_native`及 external
  response完全分离。

残余风险：

1. 本验收只冻结 RΔ。response.error v2、host projection、Router、telemetry
   admission/redaction/storage/query隔离及 cross-layer W1/S1 仍未实现或验收。
2. 后续 host sink必须保留当前 provider owner和最终 trace/error id，并继续把完整
   local stack限制在 restricted channel；不得从 fixed bytes、message/code或 generic
   response重建 diagnostic。
3. 本次按任务只运行五个聚焦测试；阶段级昂贵 gate仍由指定 owner执行。
4. 若修改 `ServiceErrorEnvelope`/`OpaqueServiceError` bytes、R0 export/import、
   diagnostic value/sink、wrapper时序、lane调用点或 provider stack scope，本证据按
   影响面失效并应重验。

## Verdict

Verdict：`PASS`。F334 RΔ冻结，F336 shared wire/telemetry checkpoint可继续；不得将
本结果表述为 W2-W、A6 或 Phase 5 PASS。
