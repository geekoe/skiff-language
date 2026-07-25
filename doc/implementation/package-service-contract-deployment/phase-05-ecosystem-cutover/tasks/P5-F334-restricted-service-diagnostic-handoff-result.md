# P5-F334 Restricted service diagnostic handoff result

状态：PASS（实现检查点完成；未 push，未承接 F335）。

## 候选与范围

- worktree：`/Users/geek/workspace/skiff-p5-f334-restricted-diagnostic`
- branch：`codex/p5-f334-restricted-diagnostic`
- 实现起点：`5ec126be9013659e57038314bae844b0da499bfe`
- production 父基线：`677305be0a0fa6f490a937fefdc7fd4e7cab1b35`
- 本节点只修改 capability-context 与 eval 的 restricted diagnostic handoff。没有修改
  `ServiceErrorEnvelope`、request/host/transport、Router、telemetry service、compiler/std 或 generic
  WebSocket，也没有接入 F335/H 的 host sink。

## 实现结果

1. `runtime/capability-context/src/telemetry.rs`新增不可序列化、clone-safe 的
   `RestrictedServiceDiagnostic`、有限`RestrictedServiceDiagnosticCauseKind`、typed owner 与独立
   `RestrictedServiceDiagnosticSink`。value只携带 provider service/operation/activation/request
   generation、最终 correlation、typed source/stack和有限 cause kind；不携带 payload/display、
   provider heap、`RuntimeValue`、`TypeAddr`或 generic attrs。
2. `TelemetryCapabilityContext`保持原`emit_native`不变，新增独立 sink builder/submit seam。默认 sink
   丢弃；recording/failing sink均通过同一 production typed value验证。
3. `CanonicalServiceErrorChannel::export_provider_failure`这个 F332 R0 owner保持原 API 和语义不变；新增
   additive wrapper，先调用 R0 固定错误，再从最终 envelope复制 correlation/kind，并在 provider heap
   借用结束前 best-effort提交 diagnostic。sink失败被明确忽略，返回同一`OpaqueServiceError`。
4. ordinary、async unary/server stream共用该 wrapper；`ContractOperation` service test effect也走同一
   handoff。成功、cancel/control以及`PackageCallable`分支不调用 wrapper。
5. 删除 ordinary 原有 test-only `OrdinaryProviderFailureRecord`/spy；所有 lane 测试只观察 production
   `RestrictedServiceDiagnosticSink`，没有第二种 record shape。

## 完成标准自验收

| # | 结果 | 证据 |
| --- | --- | --- |
| 1 typed、clone-safe、安全字段 | PASS | capability-context value无 serde derive、heap/value/type addr或开放 attrs；unit test验证 context clone后提交 |
| 2 独立 internal sink | PASS | `emit_native`未改且单独测试为失败时，restricted recording sink仍成功；默认 sink丢弃 |
| 3 lane与时序 | PASS | ordinary、async unary、server stream、ContractOperation每个失败 export一次；server cancel、成功 terminal与 PackageCallable为零 |
| 4 A→B→C | PASS | public与 private→Internal两种三跳均保持 raw bytes；C和B各一条不同 activation的本地 diagnostic，B不继承A stack |
| 5 correlation与隐私 | PASS | diagnostic correlation逐项等于最终 fixed envelope；cause kind只按 envelope enum映射，不按 message/code反推 |
| 6 sink故障不遮蔽 | PASS | failing sink前后 encoded bytes完全相同，原分类/correlation不变 |
| 7 删除平行 owner | PASS | ordinary旧 record/spy反搜为零；统一 recording sink按 request generation收集 production value |

## Exact-bytes、逐跳栈、lane 与 negative 证据

| 探针 | 结果 |
| --- | --- |
| ordinary public heap lifetime | public payload在 provider heap仍可借用时完成固定与提交；一条 diagnostic 的 source/stack和 fixed correlation精确匹配 |
| async unary | 真实 async lane返回与 ordinary相同 encoded bytes，并且只提交一条、correlation等于该 carrier |
| server stream | 真实 producer failure发布 fixed terminal且只提交一条；预先 request cancel提交零条 |
| A→B→C public/Internal | B/A import record bytes相同且最终A carrier仍相同；C stack只有C throw site，B stack只有B local site与C remote boundary |
| ContractOperation / PackageCallable | linked ContractOperation throw提交一条；真实 PackageCallable typed throw被本地 catch且提交零条 |
| sink failure | baseline与 failing-sink结果的`encoded_bytes()`相同 |
| private/source安全 | `provider-private-secret`、private file message与 source path不在 safe diagnostic debug/fixed bytes；diagnostic只保留 typed `SourceSpanRef`，不保留 path/function/message |

## 验证

以下均在最终 worktree运行：

```text
cargo test -p skiff-runtime-capability-context --lib -- --list
  35 tests, 0 benchmarks
cargo test -p skiff-runtime-capability-context --lib --no-fail-fast
  35 passed
cargo test -p skiff-runtime-eval --lib restricted_service_diagnostic -- --list
  10 tests, 0 benchmarks
cargo test -p skiff-runtime-eval --lib restricted_service_diagnostic --no-fail-fast
  10 passed
cargo test -p skiff-runtime-eval --lib service_error_consumer --no-fail-fast
  5 passed
cargo test -p skiff-runtime-eval --lib assembly_execution::async_stream_cancel --no-fail-fast
  15 passed
cargo test -p skiff-runtime-eval --lib service_error_channel_contract_operation --no-fail-fast
  4 passed
cargo check -p skiff-runtime-eval --lib
  PASS
git diff --check
  PASS
```

compiler-source/linker仍报告既有 dead-code/unused warnings；没有新增失败。按任务约束未运行完整 eval、
workspace/root、stable或 live。

## 反搜与后续

- ordinary旧 spy symbols：0。
- production diagnostic调用点只有 ordinary、shared async/stream helper与 ContractOperation materializer。
- capability value中仅现有 user telemetry API使用`serde_json::Value`；restricted value本身没有 JSON
  attrs或 serde实现。
- model/request/host/transport/router/telemetry service diff：空。
- 未决实现问题：无。本提交解除 F335 的 corrective prerequisite；默认丢弃 sink仍须由 F335/H 按既定
  restricted telemetry checkpoint接到 host，不能把它改接`emit_native`或 external response DTO。
