# P5-F440K Cancellation request / Host / transport finalization result

状态：`COMPLETED`。没有触发 `TASK_SCOPE_EXPANDED`。Root request、Host adapter 与 transport
response writer 已统一为同一个 internal cancellation terminal；取消不再生成普通 payload、catch
identity 或 `response.error`。Deadline / instruction limit 仍是普通 `TimeoutError`，活跃 caller
遇到 provider/runtime 丢失时仍得到 `ProviderUnavailableError`。

## 1. 输入、提交与写集

| 项目 | Commit | Tree |
| --- | --- | --- |
| 精确 integration 输入 | `1897df3f98c6ba1d4f8522c5003e295380ed54e3` | `483ba35924a46bf6e08052c6fb4a7c61e113535d` |
| task worktree 起点 | `6e43c531a1d8107d5bd84f99642a7649e7b1d54b` | `cf70364eb699e78f1a3294c5d6d0fcf69bdc9eaf` |
| implementation | `d1d1d174163843ac78af4c68d6c5b6611efbee9b` | `28a132b3c60518746a2da4dab5e23d3946c17232` |

task 起点相对精确输入只新增 F440K / F440L 两份任务文档。

implementation 只修改：

- `runtime/request/**`
- `runtime/host/**`
- `runtime/transport/**`

除此之外只新增本文 result。

## 2. 实现结果

### 2.1 Request 与 Host error carrier

- `RequestError` 和 Host `RuntimeError` 不再实现 `WirePayload`，各有一条 `compile_fail`
  契约证明 cancellation 不能进入 total wire API。
- 两层均显式提供：
  - `is_cancellation_terminal()`；
  - `ordinary_payload()`；
  - `ordinary_catch_projection()`；
  - ordinary response projection。
- `Cancelled`、cancelled execution budget 及带 diagnostic/source wrapper 的 eval cancellation
  均得到 `payload=None`、`catch=None`、`response=None`。
- Deadline 与 instruction limit 保持 `TimeoutError` / Timeout catch identity。
- 仍需进入动态 `WirePayload` API 的普通错误必须先经过 `OrdinaryRequestError` 或
  `OrdinaryRuntimeError`；构造器拒绝 terminal。
- Host 已删除依赖 `Box<dyn WirePayload>` downcast 识别 request/eval/native/stream cancellation
  的旧 mixed-carrier 迁移层。File、HTTP、stream、native 与 eval 转换改为结构化 terminal 分流。

### 2.2 Root terminal owner、deadline 与流式终止

- `RequestSupervisor` 以 active request 的精确实例身份原子认领 terminal；同 request id 的晚到旧
  completion 不能删除或重开新实例。
- Router `request.cancel` 在持有 active map owner 时设置显式 `cancel_requested`、唤醒 work token、
  记录预算并只发一次 `request.cancel` telemetry。
- `cancel_requested` 与 work cancellation token 已分离：
  - 前者只表示 caller/ancestor 的可观察取消；
  - 后者也可用于 deadline 赢得后终止 child/losing lane；
  - 因此 deadline cleanup 不会再被 supervisor 误判成用户取消。
- HTTP 与 WebSocket root select 固定为 `cancel -> deadline -> execution result`。执行 lane 被唤醒时
  还会重新检查同一个绝对 deadline，避免 Tokio timer-driver 调度滞后让已过期执行错误抢先形成
  `UnhandledServiceError`。
- Success、ordinary error、fixed service failure 与 cancellation 均须先取得 supervisor owner，
  晚到 lane 不写 frame。
- 流式 `response.end` 与 response-ceiling `response.error` 先暂存在 Host sink；只有 supervisor
  授予 success/ordinary-error owner 后才发送。取消获胜会丢弃暂存 terminal，不会出现
  “线上已有终止帧、telemetry 又记 cancellation”的双终止。

### 2.3 Host operation、stream 与 actor cleanup

- HTTP request/body/send、file/native operation、SSE/body stream 均继续由真实 cancellation token /
  AbortSignal 唤醒；terminal 不再经 producer `WirePayload` 降级。
- Outbound request lease 在 caller cancel、timeout、Router disconnect、writer close、stream drop
  时继续精确清 registry 与 lease；重复 terminal 只发送一次 cancel control。
- Blocked stream send/next、early break、outer/inner cancel 与 request-scope drop 继续释放 producer、
  stream state 与 child work。
- Actor method select 固定为 ancestor cancellation、owner outcome、deadline；新增真实 binary
  invoke/cancel frame测试证明：
  - cancellation 唤醒 pending invocation并释放 lease；
  - deadline 保持独立 `DeadlineExceeded` outcome，向上投影为 `TimeoutError`；
  - cancellation 与 deadline 同时 ready 时 cancellation 获胜。
- 普通 provider/runtime transport failure 保持 `ProviderUnavailableError`，不会误标为 caller cancel。

### 2.4 Transport ordinary-only response mapper

- 新增 `OrdinaryResponseErrorSource` 与 `OrdinaryResponseEvent`。
- `response_event_into_frame` 只接收 ordinary event；raw `ResponseEvent::Error` 必须经过可验证的
  ordinary source，否则 fail closed。
- Cancellation source返回 `None`，没有可构造的 `OrdinaryResponseEvent::Error`。
- Fixed service failure、`TimeoutError` 与 `std.service.ProviderUnavailableError` 仍按原 canonical
  frame编码/解码。
- `request.cancel` control frame与 bounded reason保持不变；本 leaf 未实现后续
  WebSocket JSON-RPC `connection.request.cancel`。

## 3. 测试先行与验证

### 3.1 Red evidence

production 修改前及边界审计阶段得到以下真实 red：

| Probe | Red count | Red 结果 |
| --- | ---: | --- |
| transport cancellation mapper selector | 1 failed | 旧 mapper把 cancellation `ResponseEvent::Error` 编成普通错误帧 |
| request cancellation projection selector | 1 compile-failed | `RequestError` 仍只有 total `WirePayload`，缺少 ordinary-only API；R1→R2 迁移同时暴露旧 `.payload()` / `.catch_projection()` 调用 |
| root ingress cancellation selector | 1 failed | 旧 Host 虽记录 `request.cancel`，仍发送一个普通 cancellation `response.error` |
| running deadline probe | 1 failed | deadline cleanup取消 work token后，supervisor把它误判成用户取消，等待 response 超时 |
| cancel / deadline / result race probe | 1 failed | 执行预算与 timer 同时到期时曾观察到 `UnhandledServiceError`，预期 `TimeoutError` |

最后两个 probe 直接促成“显式 root cancel 标记”和“绝对 deadline 二次仲裁”，没有用延时放宽测试。

### 3.2 Selector inventory 与 focused green

最终 `-- --list` inventory：

| Crate | 非零 selector |
| --- | ---: |
| `skiff-runtime-request` | 35 |
| `skiff-runtime-transport` | 85 |
| `skiff-runtime-host` | 293 |

Focused 结果：

| 范围 | 结果 |
| --- | --- |
| Request terminal projection | 1 passed |
| Transport cancellation rejection + Timeout/ProviderUnavailable positives | 1 passed |
| Root active cancellation / no-response / telemetry | 1 passed |
| Root success/error/deadline cancel races | 1 passed；最终 tree 连续运行 10 次均通过 |
| Running deadline `TimeoutError` | 1 passed；最终 tree 连续运行 10 次均通过 |
| Deferred stream terminal owner | 2 passed |
| Actor cancel/deadline/biased winner | 3 passed |
| Outbound cleanup selectors | 5 passed |
| Stream cleanup selectors | 25 passed |
| HTTP AbortSignal/token cancellation selectors | 3 passed |
| Service-stream child cleanup | 1 passed |
| Provider transport failure positive | 1 passed |

### 3.3 完整 matrix

| 命令 | 结果 |
| --- | --- |
| `cargo test -p skiff-runtime-request` | PASS：34 unit + 1 compile-fail doctest |
| `cargo test -p skiff-runtime-transport` | PASS：83 unit + 2 integration |
| `cargo test -p skiff-runtime-host` | 外部 fixture blocker：278 passed、4 failed；见 3.4 |
| Host lib 排除 4 个 F440L stale fixture | PASS：278 passed、4 filtered |
| `cargo test -p skiff-runtime-host --doc` | PASS：1 compile-fail doctest |
| Host `p5_f340_service_error_host` | PASS：6 passed |
| Host `p5_f345_service_error_convergence` | PASS：2 passed |
| Host `active_runtime_assembly` | 1 passed、1 个 F440L stale fixture failed；排除该 fixture 后 1 passed |
| `cargo check -p skiff-runtime-request` | PASS |
| `cargo check -p skiff-runtime-transport` | PASS |
| `cargo check -p skiff-runtime-host` | PASS |
| `cargo check -p runtime` | PASS |
| `cargo fmt --all -- --check` | PASS |
| `git diff --check` | PASS |

最终 Host inventory 293 个 selector中，288 个可直接通过；其余 5 个全部是同一个 F440L
assembly-identity fixture漂移。

### 3.4 外部完整测试 blocker

Host 的 4 个 lib failure：

- `activation_rejects_superseded_transient_service_db_wire`
- `assembly_activation_fails_closed_before_connection_bootstrap`
- `assembly_activation_reply_uses_runtime_to_router_codec`
- `binary_assembly_activation_command_uses_router_to_runtime_codec`

以及 integration failure：

- `rejected_exact_ref_preserves_committed_generation_and_two_replicas_are_independent`

都使用旧 `skiff-runtime-assembly-v1` fixture，当前 decoder要求
`skiff-runtime-assembly-v2:sha256:<64 lowercase hex>`。task 起点已经包含独立
`P5-F440L-runtime-http-gateway-current-fixture-repair.md` owner；本 leaf 未越界修改。

`cargo test -p runtime` 还被三个与本写集无关的 stale driver test构造挡住：

```text
runtime/driver/eval/tests/program_execution.rs:2573
TypeDeclIr no longer has field discriminator

runtime/driver/eval/tests/program_execution.rs:4278,4336
LinkedTypeDescriptor::Union now uses branches, not variants
```

`cargo check -p runtime` 通过，说明 production graph无该问题；本 leaf 未修改 driver test。

## 4. Root、Timeout 与 ProviderUnavailable 直接证据

`host_http_gateway_response_ceiling_cancel_and_stream_terminal_are_single_owner` 经过真实
request entry、Host adapter与transport mapper，直接断言：

- root active count：`0`；
- outbound child pending count：`0`；
- active outbound lease count：`0`；
- cancellation 后没有任何普通 response terminal frame；
- 对该 request 的 `request.cancel` telemetry：恰好 `1`。

补充证据：

- service-stream request cancel test证明 provider child清理且 peer隔离；
- stream runtime 25 个 focused selectors覆盖 blocked send/next、early break、outer/inner cancel、
  producer clone与request-scope drop；
- root race test对 success、ordinary error、deadline分别允许 cancel或ordinary lane获胜，但每个
  request最多一个 frame与一个 terminal telemetry；
- running root deadline和actor deadline均保持 `TimeoutError` 正例；
- transport mapper分别 round-trip `TimeoutError` 与
  `std.service.ProviderUnavailableError`；
- actor transport loss与HTTP client provider timeout测试证明活 caller得到 provider-unavailable，
  不会被 cancellation terminal吞掉。

## 5. Reverse search

任务要求的第一条搜索：

```text
rg -n 'CancelError|PlatformBuiltinErrorIdentity::Cancel' \
  runtime/request runtime/host runtime/transport
```

结果：`ZERO_MATCHES`，包括 tests 在内均为零。

第二条宽搜索共有 316 行、39 个文件，分类如下：

| Owner | 保留原因 |
| --- | --- |
| `runtime/request/src/{error,execution_control,execution_budget}.rs` | request结构化 terminal、预算与 ordinary-only wrapper |
| `runtime/host/src/error.rs`、`eval_capability_adapter/error.rs`、`native_projection.rs` | Host/eval/native结构化 adapter；`WirePayload` 只由已验证 ordinary wrapper实现 |
| `request_supervisor.rs`、`request_entry/assembly.rs` | root cancel owner、absolute deadline、pending stream terminal与 no-response suppression |
| HTTP/file/stream/actor/outbound modules | AbortSignal/token wake、registry/lease/stream cleanup与 control cancel |
| `runtime/transport/src/{response_mapper,protocol,actor_method}.rs` | ordinary-only response writer；保留内部 request/actor cancel control frame |
| tests | cancellation rejection、cleanup、Timeout/ProviderUnavailable正例 |

仅有的 `ResponseEvent::Error` production匹配位于 transport mapper的 fail-closed raw-event拒绝分支；
实际 writer分支接收的是 `OrdinaryResponseEvent::Error`。没有 production cancellation payload、
catch identity或普通 response materialization。

## 6. 后继 M0 blocker

`runtime/model/src/service_error.rs:57,88,107` 仍在 finite platform registry中定义、解析并打印
`PlatformBuiltinErrorIdentity::Cancel` / `CancelError`。本 leaf 的 request/Host/transport入口已经在
模型清理前 fail closed，但 M0 仍须删除该 registry member及其模型级 codec；本任务按停止规则没有
越界修改 `runtime/model/**`。

## 7. Scope 与禁令

- 没有修改 native、eval、capability-context crate、runtime/model、Router、compiler、artifact、
  scripts、其它 task/result或权威设计。
- 没有运行完整 verify、Router suite、live、instance、stable 或 chat smoke。
- 没有 merge、rebase、push、stable watch或部署操作。
- implementation 与 result 分开提交；result commit/tree由最终交付消息记录。
