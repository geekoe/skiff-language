# P5-F439A 取消公开面与内部终止 owner 审计结果

状态：`PASS — TASK_EXECUTABLE`。

本结果只冻结 owner、语义边界、实现 DAG 与验证入口，不包含实现。权威设计已经足以决定所有已发现
production 入口的行为，因此不返回 `TASK_NOT_EXECUTABLE`。

## 1. 输入、范围与结论

| 项目 | commit | tree | 说明 |
| --- | --- | --- | --- |
| 任务冻结输入 | `aacee2129934a6aebc2975293b5b4ed4b209c42f` | `617021923ad3d7072d19deecb9f41460dd2163e4` | 唯一语义输入 |
| 审计起点 | `ee4e494e4aa332f477376bb942e8fd5203e5c22f` | `9936c8a8d91853d97ccf7d6c21f9ac144975094e` | 只增加 F439/F439A/F439B/F439C 四个任务文档 |

`git diff --name-status aacee212..ee4e494e` 没有 production、test、fixture 或 script 变化，所以本审计
读取到的 production 事实与冻结输入一致。

冻结结论：

1. `CancelError` 不是标准库源文件定义，而是 compiler 直接注入的 builtin。它目前同时是 source
   spelling、File IR builtin name、runtime catch identity、service-error wire identity 和 Router HTTP
   projection code；这些同名层次错误地耦合在一起，必须全部硬切。
2. `Cancelled`、`ExecutionBudgetReason::Cancelled`、`CancellationToken`、stream/actor cancel signal 和
   `request.cancel` 是内部生命周期控制，必须保留。它们可以在 crate 边界使用内部 adapter variant，
   但不得再实现为用户可观察的普通错误。
3. ancestor/request cancellation 或 losing concurrent lane 不得：
   - 生成 `throw`/`Exception<CancelError>`；
   - 进入 `ServiceErrorEnvelope::{PlatformError,InternalError,PublicTypedError}`；
   - 被任何 `catch<E>` leaf 匹配；
   - 生成 runtime `response.error`；
   - 被 Router 投影为 HTTP 499 或其它替代性可捕获错误。
4. deadline 是当前仍在等待的调用的普通完成结果，继续生成可捕获的 `TimeoutError`。为停止下游工作而发送
   cancel signal/frame，不会把该调用的可观察结果改成 cancellation。
5. 当前双向 `request.cancel` control frame、Router pending terminal 状态机、runtime
   `CancellationToken` 和 outbound lease 已提供足够的实现骨架：
   - peer 仍拥有 pending 时，使用 control cancel；
   - 发起取消的一侧已经删除 pending 时，不再发送普通 error response；
   - late response 只被忽略；
   - `request.cancel` telemetry 至多完成一次。

## 2. 名字与语义层次

| 层次 | 当前表示 | 当前问题 | 冻结处置 |
| --- | --- | --- | --- |
| source qualified spelling | `std.error.CancelError` | 用户可 name、构造、throw、catch | 删除 compiler builtin；新源码必须报 unknown type |
| File IR spelling | `TypeRefIr::Builtin { name: "CancelError" }` | 可留存在 `Throw.payloadType`、`Catch.catchType` 及任意 type ref | artifact validation 硬拒绝旧 spelling；不提供兼容 |
| linked runtime spelling | `LinkedTypeRef::Native { name: "CancelError" }` | stale artifact 可恢复 platform catch leaf | 在 artifact admission 前拒绝；runtime 仍应 fail closed |
| service-error wire identity | `PlatformBuiltinErrorIdentity::Cancel` 经 serde 编码为 `CancelError` | cancellation 可跨 service 边界成为普通 platform error | 删除 enum variant、from/symbol 映射和 payload codec |
| ordinary runtime error code | `RuntimeErrorPayload.code == "CancelError"` | 可成为 `response.error` 并被 Router/HTTP 观察 | 所有 cancellation carrier 不再生成 ordinary payload |
| internal execution state | `ExecutionControlError::Cancelled`、各层 `*Error::Cancelled` | variant 本身合理，但目前同时实现 `WirePayload`/catch | 保留或收敛为内部 terminal adapter；不得 wire/catch/serialize |
| internal control wire | `request.cancel` + bounded reason | 已有正确的 pending/work cancellation 通道 | 保留；它不是 service error 或用户异常 |
| deadline public error | `TimeoutError` | 正确的普通、可捕获完成结果 | 保留 source、catch、service error 和 Router 504 投影 |

`std/**` 中没有 `CancelError` 或 `TimeoutError` 的源码定义；二者当前都由 compiler builtin registry
提供。此次只删除前者。

## 3. 公开面与传播面的完整 inventory

### 3.1 Definition、registration、lowering 与 artifact

| Owner / 路径 | 当前事实 | 必须修改 |
| --- | --- | --- |
| `compiler/core/src/prelude_registry.rs` | `COMPILER_BUILTIN_TYPES` 注册 name `CancelError`、symbol `std.error.CancelError`、kind `Error` | 删除这一 builtin；`TimeoutError` 保持 |
| `compiler/source/src/prelude_registry/mod.rs` | `install_compiler_builtin_types` 把所有 compiler builtin 加入 source type namespace | registry 删除后不可再解析短名或 qualified spelling |
| `compiler/source/src/type_resolution_model/catch_leaves.rs` | kind 为 `Error` 的 compiler builtin 变成 nominal `CatchLeafIdentity`；覆盖 catch、throw、rethrow 及 named-union leaf | 不增加 cancellation 特例；删除 builtin 后两种 spelling 都必须在类型解析阶段失败 |
| `compiler/lowering/src/function_lowering.rs` | 普通 throw/catch 分别写入 `ExprIr::Throw.payload_type` 和 `ExprIr::Catch.catch_type` | generic lowering 保留；不得再收到 cancellation type |
| `artifact-model/src/executable.rs` | `Throw`/`Catch` 是 generic `TypeRefIr` carrier，没有 Cancel 专用 variant | 保持 generic model；在 artifact type-ref validation 硬拒绝 legacy `CancelError` |
| `runtime/linker/src/linker/file_conversion.rs` | generic `TypeRefIr::Builtin` 无条件变成 `LinkedTypeRef::Native` | `validate_file_ir_type_refs` 已在 conversion 前调用；artifact checkpoint 应在此之前拒绝 legacy spelling |

不能只删除 registry：否则手写或旧 artifact 仍可把 `CancelError` 送入 linker/eval。语言尚未发布，不需要
兼容旧 artifact；最早 checkpoint 必须同时覆盖 compiler 与 artifact admission。

### 3.2 Runtime identity、matching 与 service serialization

| Owner / 路径 | 当前事实 | 必须修改 |
| --- | --- | --- |
| `runtime/model/src/service_error.rs` | `PlatformBuiltinErrorIdentity::Cancel` 的 serde 名、`from_symbol`、`symbol` 和 `catch_identity` 都是 `CancelError` | 删除该 finite registry member；反序列化 legacy platform envelope 必须失败 |
| `runtime/eval/src/exceptions.rs` | `collect_catch_type_leaves` 从 linked native name 恢复 platform identity；`request_exception_for_catch` 使用 `catch_projection` 后 exact-match | stale Cancel type fail closed；cancellation terminal 永远没有 catch projection |
| `runtime/eval/src/eval_context.rs` | native/runtime error 可先 materialize 为 request-local exception，再由普通 `ExprIr::Catch` 匹配 | terminal 在 materialization 之前旁路；不得构造 request exception |
| `runtime/eval/src/assembly_execution/service_error_channel.rs` | 任意 platform `catch_projection` 可编码为 `ServiceErrorEnvelope::PlatformError`；payload validator 明确接受 `Cancel` `{message}` | export/import 都拒绝 cancellation；不得降级为 `InternalError` |
| `runtime/eval/src/assembly_execution/async_stream_cancel.rs` | unary 和 stream 已用 `is_cancelled()` 在大部分 export 前短路 | 保留短路并让 channel API fail closed；避免未来新 caller 绕过 guard |
| `runtime/eval/src/assembly_execution/service_error_channel/tests.rs` | platform registry round-trip 列表直接包含 `Cancel` | 改为 legacy identity rejection；Timeout round-trip 保持 |

当前 in-process service path 已经尽量在 `export_provider_failure` 前识别 cancellation，但
`CanonicalServiceErrorChannel` 本身仍接受它。这不是安全边界：实现必须让 serializer 无法把 cancellation
编码成 platform 或 internal envelope，不能依赖所有 caller 永远记得先判断。

### 3.3 Runtime materialization 与最终 response

| Owner / 路径 | 当前内部 carrier | 当前错误公开化位置 | 冻结目标 |
| --- | --- | --- | --- |
| `runtime/capability-context/src/execution_control.rs` | `ExecutionControlError::Cancelled`、`BudgetExceeded(Cancelled)` | `payload()` 与 `catch_projection()` 生成 `CancelError` | 作为 canonical internal terminal/control owner；无普通 payload/catch |
| `runtime/capability-context/src/stream.rs` | `StreamRuntimeError::Cancelled` | `WirePayload` 生成 Cancel payload/catch | 保留 stream wake/cleanup，移除公开投影 |
| `runtime/native/src/error.rs` | `RuntimeError::Cancelled`、budget reason `Cancelled` | native payload/catch 生成 Cancel | 只作内部 adapter；deadline/instruction budget 仍走 Timeout |
| `runtime/eval/src/error.rs` | `RuntimeError::Cancelled`、递归 `is_cancelled()` | payload/catch 生成 Cancel；opaque carrier 可递归传播 | 保留递归 terminal classification，移除 payload/catch |
| `runtime/request/src/error.rs` | `RequestError::Cancelled` | `response_error()` 把它变成普通 response | completion API 必须区分 cancelled terminal 与 error |
| `runtime/host/src/capability_context/native_projection.rs` | 把 capability/stream/request/eval cancellation 收敛成 native `Cancelled` | 收敛本身正确 | 继续作为 adapter，不得重新生成 wire error |
| `runtime/host/src/error.rs` | `is_request_cancelled()` 递归 downcast 多种 carrier | classifier 只影响 telemetry 名，未阻止 response | 改成显式 terminal classification；不再靠公开 payload 名 |
| `runtime/host/src/host/request_entry/assembly.rs` | cancel select 返回 `RequestError::Cancelled` | 仍调用 `complete_error` 并发送 `ResponseEvent::Error` | cancel completion 删除 active work，记一次 cancel telemetry，不发 `response.error` |
| `runtime/transport/src/response_mapper.rs` | `ResponseEvent::Error` 无条件编码 `response.error` | Cancel payload 由此到 Router | cancellation 不得进入这个 enum branch；control cancel 使用现有 control mapper |

当前 Host 已经把 telemetry event name 选成 `request.cancel`，但随后仍发 `response.error`。这是根请求链上最
直接的 contract violation。

### 3.4 Router projection

| Owner / 路径 | 当前事实 | 必须修改 |
| --- | --- | --- |
| `router/src/protocol/runtimeProtocol.ts` | `PLATFORM_SERVICE_ERROR_IDENTITIES` 允许 `CancelError` | 从 fixed service platform identity 白名单删除 |
| `router/src/router/errors.ts` | `runtimeErrorStatus` 将普通 control error code `CancelError` 映射为 499 | 删除 499 投影；legacy control `response.error` 必须作为协议违规/非法 legacy code，不能落到 generic 500 |
| `router/src/router/runtimeDispatcher.ts` | pending owner 已统一处理 timeout、abort、client disconnect、runtime cancel、runtime disconnect；`finishPending` 先 detach 再 best-effort cancel | 保留 single-terminal/late-frame 规则；runtime `request.cancel` 是 control terminal，不是 error |
| `router/src/router/runtimeEndpoint.ts` | runtime-to-router `request.cancel` 已解码并交给 dispatcher | 保留双向 control frame |
| `router/src/protocol/envelope.ts`、`runtime/transport/src/{control_mapper,request_mapper}.rs` | 双向 typed binary `request.cancel` 与 reason 映射已存在 | 保留并补 terminal/no-error 断言 |

恶意、旧版或有 bug 的 runtime 发来 `response.error {code:"CancelError"}` 时，Router 不得恢复成 499，也不得
把它当成合法 service error。由于没有历史兼容要求，正确行为是 fail closed。

## 4. 直接 tests、tooling 与非 production 文字

以下测试直接固定了待删除的公开行为，不能被当成“只是 fixture”忽略：

| 路径 | 当前断言 | 后续归属 |
| --- | --- | --- |
| `compiler/tests/builtin_canonical_spelling.rs` | `std.error.CancelError` 编译并发射 `CancelError` | compiler/artifact checkpoint 改为两种 spelling 都拒绝 |
| `runtime/capability-context/src/lib.rs` | execution/stream cancellation payload 和 catch identity 为 Cancel | runtime terminal checkpoint |
| `runtime/native/src/error.rs` inline tests | native cancelled payload/catch 为 Cancel | runtime eval/native follower |
| `runtime/eval/src/error.rs` inline tests | eval Cancel payload/catch projection | runtime eval/native follower |
| `runtime/eval/src/assembly_execution/projection.rs` inline tests | platform registry round-trip 包含 Cancel | runtime eval/native follower |
| `runtime/eval/src/assembly_execution/service_error_channel/tests.rs` | Cancel platform identity 可 round-trip | runtime eval/native follower |
| `runtime/request/src/error.rs` inline tests | request cancel 有 platform catch projection | request/Host follower |
| `runtime/host/src/error/tests.rs` | Cancel payload 与可捕获名字一致；递归 classifier 覆盖多种 carrier | request/Host follower；保留 classifier coverage，删除公开 payload |
| `runtime/host/src/host/router_session/tests/runtime_assembly_request.rs` | root cancellation 回 `CancelError` | request/Host follower 改为 no `response.error` |
| `runtime/host/src/capability_context/stream_runtime/tests.rs` | stream Cancel payload/catch | request/Host follower改为内部 terminal |
| `runtime/driver/eval/tests/program_execution.rs` | cancellation 最终 payload code 为 Cancel | runtime eval/native follower改为 uncatchable terminal |

Router 没有直接写出 `CancelError` 的测试 fixture，但已有可复用的 focused selectors：

- `router/tests/protocol.test.ts` 的 cancel reason 与 header-only frame tests；
- `router/tests/runtime-assembly-unary-dispatch.test.ts` 的 timeout、caller abort、client disconnect、
  fixed/control error 互斥和 late terminal tests；
- `router/tests/runtime-registry-dispatch.test.ts` 的 owner 校验、timeout cancel、runtime cancel、
  runtime disconnect 和 stream callback cancellation tests。

分类结论：

- `scripts/check-skiff-source-layout.mjs` 把 `CancelError` 当作必需 compiler builtin，属于 tooling
  assertion，必须由 fixture/tooling follower 更新。
- `test-runner/**` 与 `cross-system-fixtures/**` 的精确 public spelling 搜索为零。
- `cross-system-fixtures/package-service-ecosystem/websocket-generation-lifecycle-wire.json:178`
  只把 `request.cancel` 用作错误帧类型变异；这是内部 control spelling，应保留。
- 本 leaf、父任务和本 result 中的 `CancelError` 是历史/审计文字，不是 production surface。后续 reverse
  search 必须将直接引用的设计/result 文档与明确的 negative rejection fixture 分类，而不是批量改写历史。

## 5. 真实取消链与 owner

```text
ancestor/request cancel                         losing concurrent/stream lane
  Router AbortSignal / request.cancel             eval/stream winner owns loser cancel
               \                                  /
                v                                v
        [request/work execution owner]
        runtime/host RequestSupervisor
        + runtime/capability-context CancellationToken / ExecutionControl
                         |
                         v
        [suspension and host-operation abort adapters]
        eval check_cancelled / actor executor
        native HTTP/file/time / stream CancellationSignals
                         |
                         v
        [pending and child-work cleanup owners]
        OutboundRequestLease / provider request / StreamRuntime
        actor.method.cancel / Router RuntimeDispatcher.finishPending
                         |
                         v
        [one internal terminal classification]
        Cancelled lifecycle terminal (never WirePayload/catch/service envelope)
                         |
                         v
        [work item completion]
        supervisor removes active request/lane; late result ignored
                         |
                         v
        [boundary observation]
        optional request.cancel control for a still-live peer
        + exactly-once request.cancel telemetry
        + no response.error / fixed service error / user exception
```

每一级只拥有自己的状态：

1. signal owner 决定 cancellation 已发生；
2. suspension/host adapter 负责唤醒，不决定公开错误；
3. pending/lease owner 原子地删除自己的 pending 并 best-effort 通知 child/peer；
4. execution terminal owner 终止当前 work item；
5. Host/Router boundary owner 决定是否仍有 peer pending 需要 control cancel；
6. telemetry owner记录一次 cancellation fact，不复用 error serialization。

## 6. Production 入口核对

| 入口 | 当前真实链 | 当前风险 | 冻结结果与 owner |
| --- | --- | --- | --- |
| service-to-service | `runtime/eval/assembly_execution/async_stream_cancel.rs` 的 caller execution token → provider request → service channel | 大部分路径在 serialization 前短路，但返回的 `RuntimeError::Cancelled` 仍有 catch projection；channel 本身也接受 Cancel | eval owner 将 terminal 与 service error 分流；caller `catch` 不得观察；provider request/stream cleanup 保留 |
| gateway request / client disconnect | Router abort → `RuntimeDispatcher.finishPending` → typed `request.cancel` → Host `RequestSupervisor.cancel` → root select | Host telemetry 名是 cancel，却仍发送 ordinary `response.error CancelError` | Router 先删 pending；Host 清理并只发 cancel telemetry，不发 ordinary response |
| stream consumer break | stream sink/consumer drop → `StreamRuntime` cancel signal；跨 service 时 `OutboundRequestLease::drop` 发送 cancel | stream cancellation 可通过 `StreamRuntimeError::Cancelled` 变成 catchable Cancel | stream registry/lease 继续 single-terminal cleanup；错误 adapter不再公开投影 |
| actor lane | Host actor adapter在 request cancellation/deadline 时发 `actor.method.cancel`；eval 将 Cancelled outcome 映射为 `RuntimeError::Cancelled` | cancellation 可捕获；`DeadlineExceeded` 当前被映射为 raw code `DeadlineExceeded`，而非 `TimeoutError` | actor cancel 使用同一 internal terminal；deadline 分支修为 Timeout；Router actor pending/ledger 继续拥有 exactly-once cleanup |
| native host operation | root execution token → `CancellationSignals` → HTTP/file/stream/time adapter → native/eval Cancelled | native/eval payload/catch 把 abort 公开化 | host operation 尽快 abort，随后只返回 internal terminal；operation deadline 保持 Timeout |
| runtime disconnect | Router 对 provider socket disconnect 清 pending并向仍存活 caller 返回 ProviderUnavailable；caller/source disconnect 会取消其后代 | 若一概标成 cancellation，会掩盖真正 availability failure；若一概返回 ProviderUnavailable，会让已取消 caller 观察替代错误 | 按角色区分：provider/target 消失是 ordinary ProviderUnavailable；caller/source 消失或其 ancestor cancel 是 terminal。两者共享 pending cleanup owner，但不共享公开结果 |

以上入口最终共享 `runtime/capability-context` 的 cancellation signal/execution-terminal 语义；各本地
`Cancelled` variant 只能是 adapter，不是新的公开 error owner。只修改 WebSocket 或 root gateway 路径不足以
完成任务。

## 7. 禁止边界与 TimeoutError

### 7.1 Cancellation 禁止边界

| 边界 | 必须成立的负向事实 |
| --- | --- |
| source/compiler | `CancelError` 与 `std.error.CancelError` 均不可解析；不能用于 type、constructor、throw、catch、rethrow 或 union leaf |
| artifact/linker | legacy `TypeRefIr::Builtin("CancelError")` 在 admission 阶段失败，不能形成 `LinkedTypeRef::Native` |
| eval exception | internal terminal 没有 `CatchIdentity`、identified runtime value 或 `RequestException` |
| service boundary | export 不生成任意 `ServiceErrorEnvelope`；import 不接受 legacy platform identity；不得降级为 `InternalError` |
| native/stream/actor adapter | 可以中止 Future、stream 或 lane，但不能用 `WirePayload` 把 terminal 改成普通错误 |
| request/transport | 不创建 `ResponseEvent::Error` 或 `response.error` |
| Router/gateway | fixed-service 和 control legacy Cancel code 均 fail closed；不投影 499 |
| telemetry | 可记录 `request.cancel` 和 bounded reason；不得借 telemetry error map 重新构造公开 payload |

### 7.2 Timeout 为何不同

deadline 的 observer 仍存活并等待一个普通调用结果；它需要一个可被业务选择处理的确定性结果。因此：

- `ExecutionBudgetReason::DeadlineExceeded` 和有效 operation deadline 继续投影为
  `PlatformBuiltinErrorIdentity::Timeout` / `TimeoutError`；
- timeout 可以进入普通 catch 和 service error serialization；
- deadline owner 先删除 pending，再取消 provider/child work，是 cleanup mechanism；
- cleanup 使用 cancel signal/frame 不改变 caller-visible `TimeoutError`；
- cancellation 与 deadline 同时 ready 时，ancestor cancellation 必须赢得 terminal，不能因 cleanup race
  泄漏 Timeout；deadline 已经赢得 semantic terminal 后，也不能被随后发送的 child cancel 降级为
  cancellation。

`runtime/eval/src/assembly_execution/async_stream_cancel.rs` 已有 biased cancel select，以及
`publish_provider_deadline_terminal` 防止 timeout 被 downgrade 的结构，可作为实现基准。另一个必须修复的
早期探针是 `runtime/eval/src/actor_dispatch.rs` 当前把 actor deadline 映射为 raw
`DeadlineExceeded` code。

## 8. 最小实现 DAG 与互斥写集

每个实现 leaf 除自己的唯一 result 文档外，只能写下表指定代码根。代码写集彼此不重叠；下游 task 必须从
上游 checkpoint 建立，不能并行修改同一 owner。

```text
C0 compiler/artifact hard cut
  -> R0 runtime cancellation terminal checkpoint
       -> R1 native/eval/service-channel follower
            -> R2 request/Host/transport finalization follower
            -> M0 runtime platform-error model cleanup
       -> Q0 Router pending/projection follower

R1 + R2 + M0 + Q0
  -> F0 fixture/tooling/reverse-search follower
       -> V0 single combined/final-gate owner
```

`M0` 必须等 R1/R2 不再引用 `PlatformBuiltinErrorIdentity::Cancel` 后再删除 finite enum member；Q0 可在 R0
之后独立开发，但 combined integration 必须等待 R2 与 M0。图中边表示语义/集成依赖，不授权跨写集顺手
修改。

### C0 — compiler/artifact public-surface hard cut

- 写集：`compiler/**`、`artifact-model/**`。
- 首次实际修改：先增加 failing tests，分别证明 source 的短名/qualified spelling 和手写
  `Throw.payload_type` / `Catch.catch_type` legacy File IR 被接受；随后删除 builtin 并在
  `validate_file_ir_type_refs` 硬拒绝。
- 聚焦测试：
  - `compiler/tests/builtin_canonical_spelling.rs` 的 negative cases；
  - artifact type-ref validation 的 throw/catch/union nested cases；
  - linker 调用 artifact validator 的最小 stale-artifact probe。
- 反向搜索：`compiler/**`、`artifact-model/**` production 不再注册或发射该 spelling；只允许命名明确的
  negative rejection test/tombstone。
- 最早风险探针：手写旧 artifact 在 linker conversion 前失败；不能通过 nested union、rethrow 或 test
  effect throw 绕过。

### R0 — runtime cancellation terminal checkpoint

- 依赖：C0。
- 写集：`runtime/capability-context/**`。
- 首次实际修改：给 cancellation 增加明确 internal terminal classification，并先写测试证明
  `ExecutionControlError::Cancelled`、`BudgetExceeded(Cancelled)` 和 `StreamRuntimeError::Cancelled`
  没有 catch/service/ordinary wire projection；token wake 与 cleanup 仍工作。
- 聚焦测试：execution control token、stream blocked send/next wake、outer/inner cancellation、deadline
  remains Timeout。
- 反向搜索：该根不再产生 `"CancelError"` 或 `PlatformBuiltinErrorIdentity::Cancel`。
- 最早风险探针：already-cancelled token 与 pending suspension 都立即终止；deadline reason 仍单独投影。

### R1 — native/eval/service-channel follower

- 依赖：R0。
- 写集：`runtime/native/**`、`runtime/eval/**`、`runtime/driver/**`。
- 首次实际修改：先把 service channel cancellation export test 改成“没有 envelope 的 terminal”，再删除
  native/eval Cancel payload/catch；保留递归 terminal detection。
- 聚焦测试：
  - native host-operation cancellation；
  - eval `request_exception_for_catch` 不能捕获 terminal；
  - in-process unary/stream provider cancellation 不 export/import；
  - losing stream lane cleanup；
  - actor cancel 与 actor deadline；
  - driver cancellation across task boundary。
- 反向搜索：没有 cancellation `catch_projection`、`RuntimeErrorPayload` 或
  `ServiceErrorEnvelope` 生成点；legacy spelling 只允许 fail-closed validator/negative test。
- 最早风险探针：
  - cancel、deadline、provider completion 同时 ready；
  - consumer break 与 provider error 同时发生；
  - actor cancellation 与 deadline 同时发生；
  - cancellation 不能退化成 `InternalError` 或 `ProviderUnavailableError`。

### R2 — request/Host/transport finalization follower

- 依赖：R1。
- 写集：`runtime/request/**`、`runtime/host/**`、`runtime/transport/**`。
- 首次实际修改：新增 root ingress cancellation test：active request 和 child pending 均清零、一次
  `request.cancel` telemetry、零 `response.error` frame；随后拆分 request completion 的
  cancelled/error 分支。
- 聚焦测试：
  - router-session root request cancel；
  - HTTP gateway/client disconnect；
  - outbound lease drop 与 runtime writer closed；
  - stream early break；
  - actor method cancel；
  - HTTP/file/native operation abort；
  - deadline response 仍为 `TimeoutError`。
- 反向搜索：`RequestError::Cancelled` 不调用 `response_error()`；Host cancel branch 不构造
  `ResponseEvent::Error`；transport response mapper 不接收 cancellation terminal。
- 最早风险探针：Router cancel 与 successful completion 同时发生时，最多一个 terminal，且晚到成功/error
  不重新打开 pending。

### M0 — runtime platform-error model cleanup

- 依赖：R1、R2。
- 写集：`runtime/model/**`。
- 首次实际修改：先加 legacy platform envelope decode rejection，再删除
  `PlatformBuiltinErrorIdentity::Cancel`、serde rename、`from_symbol`、`symbol` 和 catch identity。
- 聚焦测试：finite platform registry、strict service envelope decode、Timeout platform round-trip。
- 反向搜索：production 不再含 Cancel enum/member/wire identity；negative JSON fixture可保留并明确分类。
- 最早风险探针：伪造 `PlatformError { builtinErrorIdentity:"CancelError" }` 必须严格失败，不能 fallback 为
  internal error。

### Q0 — Router pending/control/projection follower

- 依赖：R0；combined integration 依赖 R2、M0。
- 写集：`router/**`。
- 首次实际修改：先加两个 negative tests：fixed-service platform Cancel identity 和 control
  `response.error CancelError` 都不能被投影；随后删除 whitelist member 与 499 mapping。
- 聚焦测试：
  - protocol validation；
  - timeout request.cancel；
  - caller abort/client disconnect；
  - runtime-originated cancel owner check；
  - runtime disconnect；
  - late/duplicate response；
  - fixed/control error channel 互斥。
- 反向搜索：Router production 不再出现 `CancelError`；`request.cancel` 与 bounded reason 保留。
- 最早风险探针：abort、timeout、runtime cancel 和 disconnect 竞争时 `finishPending` 只执行一次；provider
  disconnect 对仍活 caller 保持 `ProviderUnavailable`，不误标为 user cancellation。

### F0 — fixture/tooling/reverse-search follower

- 依赖：R1、R2、M0、Q0。
- 写集：`test-runner/**`、`cross-system-fixtures/**`、`scripts/**`。
- 首次实际修改：从 source-layout 必需 builtin 列表删除 `CancelError`，增加全链 negative inventory；
  保留 control `request.cancel` fixture。
- 聚焦测试：source-layout checker、cross-system protocol fixture checker、test-runner 的 stale
  artifact/service-envelope rejection。
- 反向搜索：对允许的所有非 doc 根做最终分类；只允许 negative rejection fixture 与历史/result 文字。
- 最早风险探针：同一 fixture 同时证明 legacy public spelling/wire identity 被拒绝，control cancel frame
  仍可双向传输。

### V0 — 唯一 combined 与昂贵 gate owner

- 依赖：F0。
- production 写集：无；只允许独立验证 result。
- 先跑 focused combined：
  - source/legacy artifact rejection；
  - service-to-service cancel；
  - gateway/client disconnect；
  - stream break；
  - actor cancel/deadline；
  - native pending operation；
  - provider runtime disconnect；
  - Timeout catch/service/Router projection。
- 最后且只由 V0 运行昂贵 gate：
  - `node scripts/verify.mjs`（完整 Rust/Node/source-layout gate）；
  - Router 完整 Vitest suite；
  - test-runner/cross-system fixture 完整 suite；
  - 如改动触发仓库约定，再由该 owner 执行相应 combined smoke。
- 本 DAG 不授权 stable/live/instance，也不授权 push。

## 9. 验证矩阵

| 检查 | 命令/方法 | 结果 |
| --- | --- | --- |
| baseline identity | `git rev-parse HEAD HEAD^{tree} aacee212^{tree}` | 起点 commit/tree 与任务一致；production delta 为零 |
| production + internal inventory | 对允许根搜索 `CancelError`、Cancel enum、各层 `Cancelled`、`request.cancel` | 256 行、61 个文件；已逐层分类 |
| exact public identity inventory | 对允许非 doc 根搜索 `CancelError\|PlatformBuiltinErrorIdentity::Cancel` | 47 行、19 个文件；production、direct test、tooling 已完整列出 |
| std source | 在 `std/**` 搜索 `CancelError\|TimeoutError` | 两者均无 std 源定义，确认 compiler-owned |
| fixture public spelling | 在 `test-runner/**`、`cross-system-fixtures/**` 精确搜索 | 0；仅发现应保留的 `request.cancel` control fixture |
| crate ownership | `cargo metadata --no-deps --format-version 1` | 定位 artifact/compiler/model/capability/native/eval/request/host/transport/linker/loader/test-runner packages |
| compiler test listing | `cargo test -p skiff-compiler --test builtin_canonical_spelling -- --list` | 2 tests、0 benchmarks，selector 非零 |
| cheap focused baseline | `cargo test -p skiff-compiler --test builtin_canonical_spelling` | 2 passed、0 failed |
| Router selector listing | `pnpm --dir router exec vitest list ...` | 本 worktree 未安装 `vitest`，命令以 254 退出；未安装依赖，改用静态 test-name 清点 |
| prohibited gates | full Rust/Router suite、`node scripts/verify.mjs`、live、instance、stable | 均未运行 |

Router test runner 不可用不影响审计可执行性：相关 selectors 已从源码静态确认，实际运行归 Q0/V0。当前
baseline compiler test 的通过只是证明现有错误公开面确实被测试固定，不是最终验收。

## 10. 最终验收不变量

实现 DAG 合流后必须同时满足：

1. 用户源码、compiler IR 和 admitted artifact 都无法 name/throw/catch `CancelError`。
2. runtime finite platform registry、service envelope 和 Router whitelist/status 中没有 Cancel identity/code。
3. 所有六类 production 入口都以同一 internal terminal 语义结束，且各自 pending/lease/stream/actor
   owner 只清理一次。
4. cancellation 不被 catch，不跨 service serialization，不产生 ordinary response。
5. deadline 在所有入口保持 `TimeoutError`；provider/target runtime disconnect 对仍活 caller 保持
   `ProviderUnavailable`。
6. 双向 `request.cancel` control frame、bounded reason、late-frame ignore 和一次性 cancel telemetry
   保持有效。
7. 最终 reverse search 对 production 为零；历史/result 文字和明确 negative fixture 被单独分类。

本审计到此结束，不自行承接任何实现 leaf。
