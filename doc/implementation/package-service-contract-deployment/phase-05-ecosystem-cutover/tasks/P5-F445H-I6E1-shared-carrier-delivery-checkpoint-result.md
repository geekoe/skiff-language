# P5-F445H-I6E1 shared invocation carrier delivery checkpoint result

状态：

```text
IMPLEMENTATION_PASS
TASK_SCOPE_EXPANDED = NO
E2_E3_E4_E5_E6_UNBLOCKED = YES
```

I6-A 已有的同一个 invocation-time `OwnedExecutionControl` 现在从 Eval native
projection/wrapper 机械交付到 HTTP、WebSocket request、time、file 与 Actor
control/method/spawn 的内部 capability seam。E1 没有实现任何下层 pending winner、timeout、
cancel、cleanup 或结果投影。

## 1. 候选身份

| 项 | commit / tree |
| --- | --- |
| task base commit | `1617299b23a1ac29ea889c573b8c49acd83785d0` |
| task base tree | `6d55ebfdf245f6bd3291e4016d59a52b1231b57a` |
| task publication HEAD | `75a800339ff947427939658851a88e18f3a2389d` |
| implementation commit | `ba66719e03cbabde2e159b94761cc1a1c71b35d2` |
| implementation tree | `0b1972158d710c4355274f7fb272be292dcc7927` |

`75a80033` 只在固定 task base 上增加 E1 合同；implementation 相对它恰好修改合同允许的
23 个 production/test/fixture 文件。

## 2. 每条 carrier path 与真实 receipt

统一内部签名在现有业务参数末尾按值接收
`skiff_runtime_capability_context::OwnedExecutionControl`。所有上游只 clone 同一个
`RuntimeNativeInvocationExecutionControl` 内的 owned façade；delivery 不调用
`execution_scope()` 或 `acquire_lease()`。

| 能力 | 实际 carrier path | receipt |
| --- | --- | --- |
| HTTP | projection current control → `RuntimeNativeHttpClientCapabilityContext` → unary/body-open/SSE native dispatch → `HttpClientCapabilityContext` → lower `HttpClientCapabilityApi`/Host adapter | `f445h_i6_carrier_delivery_receipt_http_unary_reaches_lower_api` |
| WebSocket request | projection current control → `RuntimeNativeWebsocketCapabilityContext` → public三业务参数 native call → Eval internal request API新增 owned参数 → lower request API/Host adapter | `f445h_i6_carrier_delivery_receipt_websocket_request_reaches_lower_api` |
| time | projection current control → `RuntimeNativeTimeCapabilityContext` → `NativeTimeCapability::execution_control()` 同步 getter → time dispatch可取得同一 owned carrier | `f445h_i6_carrier_delivery_receipt_time_getter_returns_current_control` |
| file | projection current control → direct/provider `RuntimeNativeFileCapability` 六项 operation，及 source-stream `next` → capability-context facade → lower API/Host adapter | `f445h_i6_carrier_delivery_receipt_file_create_reaches_lower_api` |
| Actor control | projection current control → `RuntimeNativeActorCapabilityContext` → get-or-create/replace/find/remove → `ActorClient` → lower Actor API/Host adapter | `f445h_i6_carrier_delivery_receipt_actor_find_reaches_lower_api` |
| Actor method | `prepare_actor_method` 捕获当前 `context.execution().owned()` → `PreparedActorMethodInvocation` → `invoke_actor` → Host `invoke_actor_method` | method prepared-operation fixture机械跟随并编译 |
| spawn | spawn statement捕获当前 `context.execution().owned()` → `ActorClient::submit_spawn` → Host `submit_spawn_and_wake` | canonical spawn fixture机械跟随并编译 |

五个真实 receipt case 都建立 request root、外层 derived scope 与内层 current derived
scope，再从真实 Eval native projection 取得对应 capability。记录型 lower fake实际收到 owned
control后验证：

- nesting 与 current inner scope一致；
- `EffectiveDeadline` 的绝对 `Instant`、source/site/nesting一致；
- lower receipt触发该 deadline owner时，同一个 current scope local signal同步变为 cancelled；
- operation Ready结束前后 lifecycle 均为
  `active_leases=0, active_waiters=0, active_timers=0`。

## 3. RED / GREEN

实现前先加入 HTTP unary真实 method receipt。旧代码能够编译并调用 Ready lower fake，但没有任何
carrier参数可交付：

```text
cargo test -p skiff-runtime-eval \
  f445h_i6_carrier_delivery_receipt_http_unary_reaches_lower_api -- --nocapture

1 failed / 396 filtered
left: 0 lower receipts
right: 1 expected receipt
```

这是真实行为 RED，不是编译失败。完整接线后的 selector 结果：

```text
cargo test -p skiff-runtime-eval f445h_i6_carrier_delivery_receipt -- --list
5 tests

cargo test -p skiff-runtime-eval f445h_i6_carrier_delivery_receipt -- --nocapture
5 passed / 0 failed / 396 filtered
```

listing 与 execution 数量一致，HTTP、WebSocket、time、file、Actor 五类能力各有一条真实
method/getter receipt。

## 4. 编译、格式与边界验证

| 层级 | 命令 | 结果 |
| --- | --- | --- |
| 四 crate 接线 | `cargo check -p skiff-runtime-capability-context -p skiff-runtime-native -p skiff-runtime-eval -p skiff-runtime-host --locked` | PASS；仅既有 warnings |
| Rust 格式 | `cargo fmt --check` | PASS |
| diff | `git diff --check` | PASS |

反向核对：

- Eval HTTP、WebSocket、file、Actor delegation 和 time getter均从唯一
  `RuntimeNativeInvocationExecutionControl::execution_control()` clone owned carrier；
- production新增代码没有 `acquire_lease`、scope derive、timer、sleep、deadline计算或新的
  cancellation winner；
- `dispatch/time.rs`、E4 stream、DB、HTTP ingress、Router、wire、compiler、artifact、std、
  Cargo manifest与 lockfile均无 diff；
- `requestJsonToConnection(connectionId, method, value)` 等公开 Skiff/native业务参数没有变化；
- Host adapter仅接收/持有 carrier，旧 root-token/deadline waiter行为保持原样，等待 E2–E6
  在 operation真正开始时消费。

实际写集为：

```text
runtime/capability-context/src/{http,file,actor}.rs
runtime/eval/src/capabilities.rs
runtime/eval/src/{actor_dispatch.rs,spawn_ops.rs}
runtime/eval/src/actor_dispatch/{prepared_operation.rs,prepared_operation_tests.rs}
runtime/eval/src/program_execution/execution_scope_tests.rs
runtime/eval/src/assembly_execution/ordinary/test_runtime.rs
runtime/eval/src/spawn_ops/canonical_tests.rs
runtime/eval/tests/f445h_e4r_combined/capability_harness.rs
runtime/eval/src/actor_executor/tests/actor_concurrent_continuation_tests/evaluator_actual_pending/{actor_dispatch.rs,file_create_from_stream.rs,support.rs}
runtime/native/src/{capability.rs,dispatch/prepared_tests.rs}
runtime/host/src/capability_context/native_projection.rs
runtime/host/src/eval_capability_adapter/{http.rs,file_stream.rs,websocket.rs,actor.rs,factory.rs}
```

## 5. 子 Agent 与清理

三个边界互斥分片均从固定 task base `1617299b` 建立，未继续委派：

| 分片 | source commit | 集成 |
| --- | --- | --- |
| capability-context | `47099166641fc126188579f5d1e5ed0945803b60` | 内容一致地压入 implementation commit |
| Host adapter + native time | `e89b0797b28a99c56b93d37173ee3067ca1d4ce8` | 内容一致地压入 implementation commit |
| Actor method/spawn + fixture | `b51058f30c65b418f035bc474bcbfddf38dd4142` | 内容一致地压入 implementation commit |

合流前逐文件确认三个 source branch 与 implementation tree内容一致。三个子 worktree及
`codex/p5-f445h-i6e1-capctx`、`codex/p5-f445h-i6e1-host-native`、
`codex/p5-f445h-i6e1-actor-fixtures` 临时分支均已删除。没有 merge integration、rebase 或 push。

## 6. DAG 结论

```text
E2_E3_E4_E5_E6_UNBLOCKED = YES
```

E1 只解除共享 carrier delivery prerequisite。HTTP、WebSocket request、time sleep、file 与
Actor 的 current-scope pending winner、错误投影和 cleanup仍分别由 E2–E6 拥有，当前结果不宣称
这些下层 consumer已经实现。
