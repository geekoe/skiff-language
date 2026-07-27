# P5-F440I Cancellation native / eval / service-channel follower result

状态：`COMPLETED`。没有触发 `TASK_SCOPE_EXPANDED`；合同预期的 R2 consumer break 和当前固定输入中的
M0 测试阻塞已精确记录，没有越界修复。

本 leaf 已把 capability-context 的 internal cancellation terminal 继续贯穿 native、eval、actor、
service-to-service error channel、stream adapter 和 driver 断言。取消不能再从 native/eval 形成普通
payload、catch identity、`RequestException` 或 `ServiceErrorEnvelope`。Deadline 与 instruction limit
仍是可 catch、可经 service channel 往返的 `TimeoutError`。

## 1. 输入、提交与写集

| 项目 | Commit | Tree |
| --- | --- | --- |
| 精确 integration 输入 | `e01fa01d624929bd73c163fe0ec6168f91438b91` | `a75ee80da0cb9993671f512f2bb41a720527abbb` |
| task worktree 起点 | `5e26079dbdc851e3528d3ad8dbf809cf9b7fd29c` | `23cba855509ad4509215458125d97fadcf93d686` |
| implementation | `cd20937749791c247862c91f9b894b9fc1c8e79c` | `8a52403ef7cee8223b78e4a6533843d5ce9338df` |

task 起点相对精确输入只新增 F440I 任务文档。当前 ancestry 包含已合流的 F440E / R0
implementation，不包含仍在其它 branch 的 F440H implementation。

implementation 只修改：

- `runtime/native/**`
- `runtime/eval/**`
- `runtime/driver/eval/tests/program_execution.rs`

除此之外只新增本文 result。

## 2. 实现结果

### 2.1 Native / eval ordinary 与 terminal 硬分流

- native 与 eval 的 `RuntimeError` 不再实现 `WirePayload`，各保留一条 `compile_fail` 契约。
- 两层都提供显式 `is_cancellation_terminal()`、`ordinary_payload()` 和
  `ordinary_catch_projection()`：
  - `Cancelled` 与 `ExecutionBudgetExceeded(Cancelled)` 返回 terminal，两个 ordinary projection
    都为 `None`；
  - deadline / instruction limit 继续返回 `TimeoutError` 和 Timeout catch identity。
- 需要进入现存动态 `WirePayload` API 的普通错误必须先经过 `OrdinaryRuntimeError::try_new`；构造器
  拒绝 cancellation。request-heap stream wrapper采用同样规则。
- capability execution/stream cancellation直接转成结构化 `RuntimeError::Cancelled` /
  `StreamRuntimeError::Cancelled`，不再把 mixed carrier 装进 opaque wire payload。

### 2.2 Catch 与 exception materialization

- `request_exception_for_catch` 在任何 payload、identity 或 heap materialization 之前检查 terminal，
  cancellation直接返回 `None`。
- `EvalContext` 在 ordinary catch promotion 之前再次旁路 terminal。
- linked/native platform builtin allow-list 不再接纳 stale Cancel identity；`CancelError` linked spelling
  只保留命名明确的 fail-closed negative test。
- 独立 integration test 直接证明：带 diagnostic wrapper 的 cancellation不能形成 source catch
  exception，而同一入口的 deadline仍形成 Timeout exception。

### 2.3 Service error channel

- `CanonicalServiceErrorChannel::export_provider_failure` 自身先检查 cancellation并返回独立 terminal；
  不调用 correlation allocator，也不产生 envelope、frame、`InternalError` 或 provider-unavailable
  fallback。
- local exception、platform payload encode 和 caller import 均使用有限 allow-list；旧
  `PlatformBuiltinErrorIdentity::Cancel` 在 encode/import 两端都 fail closed。
- Timeout validator只接受 `deadlineExceeded` 与 `instructionLimitExceeded`，拒绝旧
  `"cancelled"` timeout reason。
- 正例仍覆盖 platform Timeout 的 canonical encode/decode 与 service envelope round-trip。

### 2.4 Stream、actor 与 driver boundary

- 所有 eval stream producer error 都经过 ordinary-vs-terminal adapter；普通 producer error进入
  `WirePayload`，cancellation进入结构化 `StreamRuntimeError::Cancelled`。
- `async_stream_cancel` 原有 biased winner、losing-lane cancel、consumer break、provider completion、
  deadline publication和single-terminal结构保持不变；相关 race/cleanup tests继续通过。
- actor cancellation使用同一 terminal；actor deadline从 raw `DeadlineExceeded` 改为普通
  `ExecutionBudgetExceeded(DeadlineExceeded)`，因而投影为 `TimeoutError`。
- driver task-boundary tests 已改为断言 cancellation terminal且无 payload/catch；deadline继续断言
  Timeout。它们的实际执行被下游 R2 compile break挡住，未伪报为已运行。

## 3. 测试先行与验证

### 3.1 Red evidence

production 修改前先落最终测试：

| 命令 | Red 结果 |
| --- | --- |
| `cargo test -p skiff-runtime-native error::tests::native_ordinary_projection_excludes_cancellation_and_keeps_timeouts -- --exact` | FAIL：21 个 `E0599`；`ordinary_payload`、`ordinary_catch_projection`、`is_cancellation_terminal` 尚不存在 |
| `cargo test -p skiff-runtime-eval assembly_execution::service_error_channel::tests::cancellation_export_is_terminal_and_produces_no_service_envelope -- --exact` | FAIL：`E0277`；旧 `runtime/eval/src/error.rs:999` 仍把 `ExecutionControlError::Cancelled` 装成 `WirePayload` |

两条都是非零真实 selector，不是 skip。

### 3.2 Green matrix

| 命令 | 结果 |
| --- | --- |
| `cargo test -p skiff-runtime-native` | PASS：96 unit + 1 compile-fail doctest |
| `cargo test -p skiff-runtime-eval --test catch_fixture_closure` | PASS：4 passed；包含 cancel不可 catch / deadline可 catch 正反例 |
| `cargo test -p skiff-runtime-eval --test representation_wrap_consumer` | PASS：6 passed |
| `cargo test -p skiff-runtime-eval --doc` | PASS：1 compile-fail doctest |
| `cargo check -p skiff-runtime-native` | PASS |
| `cargo check -p skiff-runtime-eval` | PASS |
| `cargo fmt --all -- --check` | PASS |
| `git diff --check` | PASS |

为了确认被一个无关旧测试构造器遮住的 eval unit matrix，本地曾只把
`runtime_http_gateway/tests.rs:384` 临时改为显式解包当前 `Option<PackageCallableId>`，运行
`cargo test -p skiff-runtime-eval --lib`：`207 passed, 0 failed`。其中直接通过：

- service channel cancellation无 envelope与 legacy Cancel encode/import rejection；
- cancellation不能形成 request exception；
- actor cancel terminal / actor deadline Timeout；
- request cancel、expired deadline与ready provider同时 ready时 cancel winner；
- consumer/request cancel、normal provider completion、late cancellation不降级 deadline；
- provider deadline以 typed Timeout到达 pending consumer。

临时适配已立即恢复，未进入 implementation commit；因此上表只把最终未修改 tree 上可直接复现的
11 个 eval integration/doctest计作常规 green。

### 3.3 完整 crate 阻塞

最终 tree 上 `cargo test -p skiff-runtime-eval` 按合同记录为外部 compile blocker：

```text
runtime/eval/src/runtime_http_gateway/tests.rs:384:50
error[E0599]: no method named `as_str` found for Option<PackageCallableId>
```

这是当前固定输入已经存在的 stale M0 test owner/blocker。它不由本 branch 的 F440H implementation
引入——本 branch ancestry 根本不包含 F440H implementation。本 leaf 不提交该无关修复。

Cargo inventory 证明 driver package真实名称是 `runtime`，不存在 `skiff-runtime-driver` package。
`cargo test -p runtime` 与 `cargo check -p runtime` 均按预期停在 R2：

```text
runtime/request/src/error.rs:148:40
error[E0599]: eval RuntimeError has no method payload

runtime/request/src/error.rs:193:40
error[E0599]: eval RuntimeError has no method catch_projection
```

因此 driver unit matrix尚未执行；这正是删除 eval total ordinary projection后要求 R2接手的 compile
checkpoint，不在本 leaf 越界改 request/Host。

## 4. Cancellation 无公开 materialization 的直接证据

| 边界 | 直接事实 |
| --- | --- |
| native / eval trait | 两个 `compile_fail` doctest证明 `RuntimeError::Cancelled` 不能作为 `WirePayload` |
| ordinary projection | cancellation及其 diagnostic/source wrapper均得到 `payload=None`、`catch=None` |
| source catch | `request_exception_for_catch` 返回 `None`，不分配 identified exception value |
| stream carrier | ordinary wrapper和request-heap wrapper都拒绝 terminal；producer使用结构化 Cancelled variant |
| service export | 返回 `Err(RuntimeError::Cancelled)`；correlation closure若被调用会 panic，测试通过 |
| service import | legacy Cancel platform envelope解码为协议失败，不恢复业务错误 |
| actor | cancel为同一 terminal；deadline为 `TimeoutError` / Timeout catch |
| driver assertions | time sleep与emit/stream task-boundary均要求 terminal且无 payload/catch |

## 5. Reverse search

对任务要求的 production roots执行：

```text
rg -n 'CancelError|PlatformBuiltinErrorIdentity::Cancel' runtime/native runtime/eval runtime/driver
```

只剩三处、全部是命名明确的 negative test：

| 路径 | 分类 |
| --- | --- |
| `runtime/eval/src/assembly_execution/projection.rs:699,704` | stale linked `CancelError` spelling必须 fail closed |
| `runtime/eval/src/assembly_execution/service_error_channel/tests.rs:127` | runtime/model尚未清理前，legacy Cancel identity的 encode/import rejection |

production 执行路径为 `ZERO_MATCHES`。

第二条宽搜索的 owner 分类如下：

| Owner | 保留原因 |
| --- | --- |
| `runtime/native/src/error.rs`、`dispatch/file.rs` | native结构化 terminal、ordinary-only carrier和 capability adapter |
| `runtime/eval/src/error.rs`、`exceptions.rs`、`eval_context.rs` | canonical eval terminal、ordinary projection与 catch前旁路 |
| `actor_dispatch.rs`、`actor_executor.rs`、`capabilities.rs` | actor/capability内部 outcome与结构化 adapter |
| `async_stream_cancel.rs`、`program_{invocation,stream}.rs`、`service_dispatch.rs`、`env.rs` | biased winner、stream cleanup、sink/producer terminal传播 |
| `service_error_channel.rs` | cancellation serialization guard；其余 `ServiceErrorEnvelope` 仅处理 ordinary/fixed failure |
| service convergence / ordinary / boundary tests | 非取消 service envelope正例与严格性 fixture |
| `runtime/driver/eval/tests/program_execution.rs` | task-boundary cancellation / Timeout最终断言 |
| `runtime_http_gateway/tests.rs` | 既有内部 cancellation cleanup测试；没有 payload、catch或 envelope |

没有 production `CancelError`、Cancel platform identity或 cancellation service envelope。

## 6. R2 与 M0 精确交接

### R2 request / Host / transport

- `runtime/request/src/error.rs:118-193`
  - `RequestError::Cancelled` 仍生成 `CancelError` payload和 Cancel catch identity；
  - `RequestError::Eval` 仍调用已删除的 eval `payload()` / `catch_projection()`，形成当前 compile
    checkpoint。
- `runtime/host/src/error.rs:344-365,393-407,462-473,623-718`
  - eval/capability/native cancellation仍经过 old opaque boxing与 mixed-carrier downcast classifier；
  - R2必须改成显式 terminal result，并让 root request completion不发送普通 response。
- `runtime/host/src/eval_capability_adapter/error.rs:16-32` 及其旧 payload/catch tests仍把 root error折回
  eval opaque ordinary error。
- `runtime/host/src/capability_context/native_projection.rs:161-300` 仍保留 old dynamic wire downcast
  迁移层；结构化 control mapping可以保留，依赖 `WirePayload` 的分支必须删除。
- `runtime/host/src/capability_context/stream_runtime/tests.rs:695-705` 仍尝试把 eval cancellation作为
  producer wire error，并断言 `CancelError`。
- `runtime/host/src/host/request_entry/assembly.rs` 与 request-entry completion是 no-response
  suppression owner；transport只应接收 ordinary `ResponseEvent::Error`，cancellation不得到达
  `runtime/transport/src/response_mapper.rs`。

### M0 runtime/model

- `runtime/model/src/service_error.rs:57,88,107` 仍定义、解析并打印
  `PlatformBuiltinErrorIdentity::Cancel` / `CancelError`。本 leaf 的 eval channel已在模型清理前
  fail closed；M0仍须删除该 public finite registry member。
- 上述 `runtime_http_gateway/tests.rs:384` 是当前固定输入的另一个 stale M0 test blocker；不得误归因
  于未合流的 F440H implementation。

## 7. Scope 与禁令

- 没有修改 capability-context、request、Host、transport、runtime/model、Router、compiler、
  artifact、scripts或 fixture。
- 没有运行完整 verify、Router、live、instance、stable 或 chat smoke。
- 没有 merge、rebase、push 或 stable watch操作。
- implementation 与 result 分开提交；result commit/tree由交付消息记录。
