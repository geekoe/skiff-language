# P5-F445H-O2 Outbound service and Actor prepared operation result

状态：`IMPLEMENTATION_COMPLETE / FOCUSED_GREEN`。

本节点已把 outbound unary、remote interface operation 与 Actor method invocation 切成明确的
prepare、heap-free wait、finalize 三阶段。wait future 只拥有跨挂起点所需的 owned state，不借用
caller heap 或 env；E4R 可以在 future 存活期间独立访问 caller heap，并只在 future 完成后执行
finalize。

`serverStream` setup 仍是同步步骤：prepare 直接返回接管 lease/receiver 的 stream value，真正的
等待仍发生在后续 `stream.next()`。普通 service dependency 与 remote interface operation 共同
进入同一个 prepared owner，没有复制请求协议或状态机。

本节点没有修改 evaluator call site、`eval_context.rs`、Actor executor/store、assembly、native、
host、service-db 或 manifest，也没有运行 stable/live/network 验证。

## 1. 输入、分支与写集

| 项 | 值 |
| --- | --- |
| production prerequisite | `d39ad5b01d1c8dc506a97034d7813d7122e670bc` |
| task document | `87e859115ba62526eb91e729c092b56e55034a0c` |
| implementation | `956c2963` |
| branch | `codex/p5-f445h-o2-outbound-actor` |
| worktree | `/Users/geek/workspace/skiff-p5-f445h-o2-outbound-actor` |

implementation 写集精确为：

- `runtime/eval/src/service_dispatch.rs`
- `runtime/eval/src/service_dispatch/prepared_operation.rs`
- `runtime/eval/src/service_dispatch/prepared_operation_tests.rs`
- `runtime/eval/src/service_dispatch/prepared_operation_tests/fixture.rs`
- `runtime/eval/src/actor_dispatch.rs`
- `runtime/eval/src/actor_dispatch/prepared_operation.rs`
- `runtime/eval/src/actor_dispatch/prepared_operation_tests.rs`

除此之外只新增本文。

## 2. Test-first 证据

先在 focused tests 中引用尚不存在的 prepared owner，再分别运行：

```text
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o2-outbound-actor/build/cargo-target \
  cargo test -p skiff-runtime-eval service_dispatch -- --nocapture

CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o2-outbound-actor/build/cargo-target \
  cargo test -p skiff-runtime-eval actor_dispatch -- --nocapture
```

两次均得到预期 RED，exit `101`。service RED 来自缺失
`PreparedOutboundUnaryOperation`/`PreparedOutboundServiceCall`；Actor RED 来自缺失
`PreparedActorMethodInvocation`。这些失败直接证明三阶段 owner 尚未实现，不来自旧测试、外部
服务或写集外组件。随后才加入 production implementation。

## 3. Outbound service owner

两个入口分别解析自己的 dispatch：

- `prepare_outbound_service`
- `prepare_outbound_service_operation`

解析后都进入 `prepare_outbound_service_request`。共同 prepare 阶段完成 mode/deadline 校验、在
caller heap 中编码 request，并同步调用一次 `start_request`。

返回值 `PreparedOutboundServiceCall` 明确区分：

- `Ready(RuntimeValue)`：当前只用于同步完成 setup 的 `serverStream`；
- `ExternalWait(PreparedOutboundUnaryOperation)`：用于 unary service call 和 remote interface
  operation。

`PreparedOutboundUnaryOperation` 只持有 owned `OutboundServiceContext`、dispatch、
`OutboundRequestLease` 和 response receiver。其
`into_wait()` 返回 `Future + Send + 'static`，wait 中没有 caller heap/env 引用。
`OutboundServiceUnaryCompletion::finalize(heap, env)` 才 decode/coerce response，并执行既有
stream-sink cancellation 检查。

lease settlement 保持单一 owner：

- `End`、fixed service failure 和 terminal error 完成 lease；
- response channel failure 与意外 stream frame 取消 lease；
- pending wait 被 drop 时由既有 lease RAII 取消；
- receiver/lease 已被 settlement 或 drop 后，late response 无法写入 caller。

finalize 在 decode 前建立 heap checkpoint；decode、coerce 或 cancellation check 失败时回滚到
checkpoint，不留下部分分配。

`serverStream` prepare 同步构造 source-backed stream value，并把 lease/receiver 所有权交给
source。它不会伪装成 external wait，也不会提前释放 lease。

## 4. Actor invocation owner

`prepare_actor_method` 在 caller 同步阶段完成 receiver、method、arity 与 plan 校验，编码 owned
arguments，构造 `ActorInvocationRequest`，并捕获 `OwnedActorCapabilityContext`。

`PreparedActorMethodInvocation::into_wait()` 返回 `Future + Send + 'static`。wait 只持 owned
capability context、request、return plan、method name 与 timeout；它启动一次 invocation，不借用
caller heap。

`ActorMethodInvocationCompletion::finalize(heap)` 在 resume 后处理：

- returned payload 的 JSON decode、boundary import 与 return-plan carrier coercion；
- cancellation/timeout 映射；
- Actor error 映射；
- capability/transport error 映射。

returned payload finalize 同样使用 heap checkpoint，decode/import/coerce 失败会回滚所有部分
写入。pending invocation future 被 drop 时，唯一 invocation owner 被取消；测试同时验证取消只
发生一次，之后到达的结果不会进入 caller。replacement/stale epoch、Actor error 与 cancellation
仍沿既有错误语义返回。

原有 `call_outbound_service`、`call_outbound_service_operation` 和
`dispatch_actor_method` 只保留为薄 prepare→wait→finalize 组合，没有第二套协议实现。

## 5. 验收矩阵

| 合同 | production 证据 | focused 测试证据 |
| --- | --- | --- |
| buffered/立即 unary response，副作用一次 | outbound prepare 同步 `start_request`，owned wait 接收 | `outbound_buffered_response_wait_is_static_and_starts_once` |
| pending wait 不借/不写 caller heap | `into_wait() -> Future + Send + 'static` | `outbound_pending_wait_does_not_borrow_or_write_the_caller_heap` 在 wait pending 时独立 mutation heap，finalize 前无写入 |
| unary settlement 与 late response 隔离 | completion/cancel/RAII 单 owner | `outbound_unary_error_and_drop_settle_the_lease_exactly_once` |
| serverStream setup 同步 | `PreparedOutboundServiceCall::Ready`，source 接管 lease | `outbound_server_stream_setup_is_a_synchronous_ready_step` |
| dependency/remote 共用 owner | 两个 prepare entry 进入同一 request helper 与 enum | `dependency_and_remote_interface_entries_share_the_prepared_owner_contract` |
| outbound finalize 失败原子性 | heap checkpoint/rollback | `outbound_finalize_heap_failure_rolls_back_partial_decode` |
| Actor Ready/Pending 与副作用一次 | owned invocation request/context | `actor_ready_invocation_wait_is_static_and_starts_once`、`actor_pending_wait_does_not_borrow_or_write_the_caller_heap` |
| Actor error/cancel/replacement 映射 | completion finalize | `actor_cancel_error_and_replacement_are_finalized_after_the_wait` |
| Actor pending drop 与 late result 隔离 | invocation future 唯一 owner | `dropping_pending_actor_wait_cancels_the_single_invocation_owner` |
| Actor finalize 失败原子性 | heap checkpoint/rollback | `actor_finalize_heap_failure_rolls_back_partial_decode` |

两个 wait API 的 `Send + 'static` 编译期断言和 pending 期间 caller heap 独立 mutation 测试共同证明：
跨 actual-Pending 生命周期的 future 不持有 caller heap/env borrow，而不只是依赖代码审阅。

## 6. 验证

所有 Cargo 命令使用 worktree 专属 target：

```text
/Users/geek/workspace/skiff-p5-f445h-o2-outbound-actor/build/cargo-target
```

| 命令 | 结果 |
| --- | --- |
| `cargo test -p skiff-runtime-eval service_dispatch -- --nocapture` | PASS：实际执行 12/12 unit tests；其它 test binary 为 0 个匹配测试，不计作证据 |
| `cargo test -p skiff-runtime-eval actor_dispatch -- --nocapture` | PASS：实际执行 6/6 unit tests；其它 test binary 为 0 个匹配测试，不计作证据 |
| `cargo check -p skiff-runtime-eval --locked` | PASS |
| `cargo fmt --check` | PASS |
| `git diff --check` | PASS |

输出只有既有 compiler/linker dead-code、ordinary test unused import，以及
`service_error_channel.rs` unreachable-pattern warning；本节点没有新增 warning 或既有失败。

禁止写集检查：

```text
git diff 87e85911..956c2963 -- \
  runtime/eval/src/eval_context.rs \
  runtime/eval/src/actor_executor.rs \
  runtime/eval/src/actor_executor \
  runtime/eval/src/actor_instance.rs
```

结果为空。implementation commit 的文件列表也只有第 1 节列出的七个允许路径。

## 7. E4R 后继接口

outbound call site 可以直接采用：

1. 调用对应 `prepare_outbound_service*`；
2. `Ready(value)` 在当前同步 segment 内继续；
3. `ExternalWait(operation)` 把 `operation.into_wait()` 交给 actual-Pending seam；
4. resume 后调用 `completion.finalize(heap, env)`。

Actor call site 可以直接采用：

1. `prepare_actor_method(context, plan, values)`；
2. 把 `prepared.into_wait()` 交给 actual-Pending seam；
3. resume 后调用 `completion.finalize(heap)`。

E4R 不需要复制 outbound lease、response protocol、Actor request encoding 或错误映射，也不需要在
wait 存活期间保留 caller heap/env borrow。evaluator call-site 迁移和 concurrent continuation
接线仍由 E4R 独占。
