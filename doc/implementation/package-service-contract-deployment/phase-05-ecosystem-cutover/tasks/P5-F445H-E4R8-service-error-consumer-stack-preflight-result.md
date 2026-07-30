# P5-F445H-E4R8 ordinary service-error consumer stack preflight result

状态：

```text
READY_FOR_E4R8_FIX
USER_DECISION_REQUIRED = NO
```

默认 libtest worker 栈 blocker 已收敛为 activation-relative unary service call 的一个
private evaluator stack-shape 问题。`PreparedActivationRelativeServiceCall::wait` 的
3,776-byte concrete future 在
`EvalContext::eval_activation_relative_service_call` 中未经过 heap indirection，直接进入
generic `await_actual_pending -> await_operation -> ActorExecutionFrame::await_if_pending` 链；
对应 async state 依次放大到 21,536、31,336、41,104 bytes。

目标 exact test 单独运行和显式 `--test-threads=1` 都稳定 stack overflow / `SIGABRT`。
`RUST_MIN_STACK=2.421875 MiB` 仍失败，`2.4375 MiB` 即通过并在约 `0.01s` 内结束，证明这是
有限 debug async/poll state 峰值，不是无限递归。

触发峰值的是 test 中第一个 linked-public exact-catch case 的 first-poll-Ready unary wait；
在第一个 `execute_internal(...).await` 后设置的只读 debugger breakpoint 未被命中。后续
unlinked catch miss 和 InternalError catch 尚未开始，因而不是触发该次 overflow 的必要条件。
相邻的单一 linked-public、无 catch case 在默认栈也独立 overflow，说明 test 串行组合会增加
余量压力，但拆 test 不能修复 production route。

唯一修复 owner 是：

```text
runtime/eval/src/eval_context/actual_pending/activation.rs
  EvalContext::eval_activation_relative_service_call
  operation.wait() -> await_actual_pending(...) call-site
```

最小修复是在该 call-site 对同一个 wait future 做 private `Box::pin`，再进入 E3 generic
owner。不修改 prepared service owner、service-error import、catch projection、test、公共 API
或通用 actual-Pending owner。

## 1. 候选身份与只读边界

| 项 | 值 |
| --- | --- |
| 诊断开始时 HEAD | `5c86c87325d74b34bb5c1a828ab3bf5effa7604f` |
| 诊断开始时 tree | `3d6c0a80f8cea5166f4e0f7a3bfecb76192969d3` |
| 冻结 production/tests commit | `464a3319b153527d5d33093d52ea6af97b6f997b` |
| 冻结 production/tests tree | `17ae8ebe6bb05202d9b3992b812cc3f60fbd8ded` |
| worktree | `/Users/geek/workspace/skiff-p5-f445h-e4r8-preflight` |
| branch | `codex/p5-f445h-e4r8-preflight` |
| 独立 target | `/Users/geek/workspace/skiff-p5-f445h-e4r8-preflight/build/cargo-target` |

`464a3319..5c86c873` 只新增 E4R7、E4R8 两份 task；对 `runtime/eval/src`、
`runtime/eval/tests`、Cargo manifests 和 lockfile 的 diff 为空。诊断开始及结束前
`git status --short --branch` 都只输出 branch header。所有默认栈命令都显式
`env -u RUST_MIN_STACK -u RUSTFLAGS`，并设置 `CARGO_NET_OFFLINE=true`；没有访问 network。

本任务只执行 exact/focused test、一次 no-run type-size 编译和只读静态/debugger探针。
没有运行完整 lib/eval，没有启动 stable/live，没有访问 MongoDB 或其它仓库，没有修改
production/tests/fixture/manifest/lockfile，也没有 merge、rebase、push 或派子 Agent。

## 2. Exact、线程与栈阈值矩阵

以下缩写用于本节：

```text
TARGET=/Users/geek/workspace/skiff-p5-f445h-e4r8-preflight/build/cargo-target
EXACT=assembly_execution::ordinary::tests::service_error_consumer::ordinary_exact_public_and_internal_catches_hit_while_unlinked_catch_misses
PUBLIC=assembly_execution::ordinary::tests::service_error_consumer::restricted_service_diagnostic_ordinary_exports_before_provider_heap_drop
ACTIVATION_FAILURE=eval_context::actual_pending::activation::activation_tests::f445h_e4r_stream_activation_unary_actual_evaluator_imports_provider_failure_once
```

### 2.1 目标 exact 与线程数

| 命令条件 | exit | 结果 |
| --- | ---: | --- |
| `EXACT -- --exact --nocapture`，默认栈/默认 test threads | `101` | `running 1 test`；stack overflow，`SIGABRT`；`394 filtered`。 |
| `EXACT -- --exact --nocapture --test-threads=1`，默认栈 | `101` | 同一 exact test 独立串行 stack overflow，`SIGABRT`。 |

命令形状为：

```bash
env -u RUST_MIN_STACK -u RUSTFLAGS \
  CARGO_NET_OFFLINE=true CARGO_TARGET_DIR=$TARGET \
  cargo test -p skiff-runtime-eval --locked --lib "$EXACT" -- \
  --exact --nocapture [--test-threads=1]
```

因此 failure 不依赖 full-suite 顺序、其它 test、libtest 并发或共享 fixture 污染。

### 2.2 有限栈阈值

以下均为同一个 `EXACT`、`--exact --nocapture --test-threads=1`，只改变
`RUST_MIN_STACK`。该变量仅用于诊断，不是允许的修复。

| `RUST_MIN_STACK` | MiB | exit | 结果 |
| ---: | ---: | ---: | --- |
| 未设置 | 默认 | `101` | stack overflow / `SIGABRT` |
| `2359296` | `2.25` | `101` | stack overflow / `SIGABRT` |
| `2490368` | `2.375` | `101` | stack overflow / `SIGABRT` |
| `2523136` | `2.40625` | `101` | stack overflow / `SIGABRT` |
| `2539520` | `2.421875` | `101` | stack overflow / `SIGABRT` |
| `2555904` | `2.4375` | `0` | `1 passed / 394 filtered`，约 `0.01s` |
| `2621440` | `2.5` | `0` | `1 passed / 394 filtered`，约 `0.01s` |

16 KiB 宽的稳定翻转区间，以及通过后立即完整执行 public/internal/unlinked 三个 case，排除
无限递归、fixture 自循环和永不完成的 Pending。它只说明有限栈峰值，不授权提高默认测试栈。

### 2.3 相邻对照

| 对照 | 条件 | exit | 结果 |
| --- | --- | ---: | --- |
| `PUBLIC`：单一 linked-public、无 catch service-error import | 默认栈，`--test-threads=1` | `101` | 独立 stack overflow / `SIGABRT` |
| 同一 `PUBLIC` | `2.125 MiB` | `101` | stack overflow / `SIGABRT` |
| 同一 `PUBLIC` | `2.1875 MiB` | `0` | `1 passed / 394 filtered` |
| 同一 `PUBLIC` | `2.25 MiB` | `0` | `1 passed / 394 filtered` |
| `ACTIVATION_FAILURE`：较小 activation-relative provider-error evaluator | 默认栈，`--test-threads=1` | `0` | `1 passed / 394 filtered` |

`PUBLIC` 证明即使没有 exact catch、后续 unlinked/internal case或 test内三次串行 execution，
同一 ordinary service-error production route仍会越过默认栈。目标 test 的更高阈值则说明
exact-catch evaluator层和组合 test poll形状进一步消耗余量；它们是放大条件，不是可以通过
拆 test 消除的根因。

## 3. 精确子路径：first-Ready linked-public

目标 test 源码在一个 `#[tokio::test]` future 中依次执行：

1. linked provider public record + exact public catch；
2. unlinked provider public record + unrelated catch miss；
3. linked provider private record经 fixed `InternalError` + exact std catch。

只读 LLDB 在同一次 type-size build产生的 test binary中，把 breakpoint 设在第一条
`linked.execute_internal(...).await` 后的 `service_error_consumer.rs:434`。默认栈直接运行
`EXACT --exact --nocapture --test-threads=1` 时 breakpoint没有命中；进程先在
`EvalContext::eval_program_expr` poll中触发 stack guard `EXC_BAD_ACCESS`。该探针只用于定位
第一个未完成的 case，没有 test summary，也不作为 GREEN。

第一个 case 的 runtime形状为：

- contract是 unary；`PreparedActivationRelativeServiceCall::ready_result` 对 unary返回
  `Err(operation)`，所以 evaluator进入 owned wait；
- provider executable只有 `Literal -> Construct -> Throw`，`may_suspend: false`，没有 sleep、
  channel、stream、callback或再次 service call；
- execution scope没有 terminal，provider future首次 poll即完成为 provider error；
- fixture没有 caller `ActorExecutionFrame`，因此 `await_operation` 运行 `None => future.await`
  分支；即使在 Actor caller中，同一个 future也是 first-Ready，不应 release/reacquire segment。

因此 blocker不是 actual-Pending、deadline或 reacquire路径，而是 first-poll-Ready concrete
unary evaluator future在 generic链中的 debug stack布局。public import和 exact catch决定可观察
结果，但栈峰值出现在第一条 service call尚未返回时；unlinked miss、InternalError import和
rethrow断言尚未执行。

## 4. 静态调用链与 async state

精确静态路径为：

```text
service_error_consumer.rs:425 target test
  -> ServiceErrorConsumerFixture::execute_internal
  -> Interpreter::execute_runtime_assembly_addr
  -> Interpreter::call_program_executable
  -> Interpreter::call_program_executable_carriers #[async_recursion]
  -> Interpreter::call_assembly_executable
  -> Interpreter::exec_program_executable
  -> EvalContext::exec_program_executable
  -> EvalContext::eval_program_catch
  -> EvalContext::eval_program_expr_ref / eval_program_expr #[async_recursion]
  -> EvalContext::eval_program_call
  -> EvalContext::eval_activation_relative_service_call
  -> EvalContext::prepare_activation_relative_service_call
  -> prepare_provider_unary
  -> PreparedActivationRelativeServiceCall::wait
  -> EvalContext::await_actual_pending
  -> actual_pending::await_operation
  -> [generic ActorExecutionFrame::await_if_pending branch is present in the type;
      this fixture executes the no-frame direct-await branch]
  -> PreparedProviderUnary::wait
  -> await_provider_unary / current_scope::wait
  -> Interpreter::call_program_executable
  -> provider Literal -> Construct -> Throw
  -> CompletedProviderUnary::finalize
  -> export_provider_failure -> FixedServiceFailure
  -> CompletedActivationRelativeServiceCall::finalize
  -> finish_activation_relative_service_result
  -> CanonicalServiceErrorChannel::import_caller_failure
  -> import_public_error
  -> materialize_service_error_local_value
  -> UserException
  -> EvalContext::eval_program_catch exact identity projection
```

任务允许的一次 no-run type-size命令为：

```bash
RUSTC_BOOTSTRAP=1 RUSTFLAGS=-Zprint-type-sizes \
  CARGO_NET_OFFLINE=true CARGO_TARGET_DIR=$TARGET \
  cargo test -p skiff-runtime-eval --locked --lib --no-run
```

编译 exit `0`。与本路径直接相关的 debug async state为：

| state/type | 大小 |
| --- | ---: |
| `PreparedProviderUnary` | `1,792 B` |
| `PreparedActivationRelativeServiceCall` | `1,888 B` |
| `ServiceErrorConsumerFixture::execute_internal` future | `2,592 B` |
| `Interpreter::execute_runtime_assembly_addr` future | `2,384 B` |
| `Interpreter::exec_program_executable` future | `1,856 B` |
| `PreparedActivationRelativeServiceCall::wait` concrete async block | `3,776 B` |
| `ActorExecutionFrame::await_if_pending<activation wait>` | `21,536 B` |
| `actual_pending::await_operation<activation wait>` | `31,336 B` |
| `EvalContext::await_actual_pending<activation wait>` | `41,104 B` |
| `EvalContext::eval_activation_relative_service_call` | `41,192 B` |
| `EvalContext::eval_program_call` | `41,272 B` |

3,776-byte wait及包含它的 poll-once/branch state在 generic future中被多处保留；
`31,336 - 21,536 = 9,800`、`41,104 - 31,336 = 9,768` 是外层继续保留 wait-bearing
nested state的layout增量，不是 wait本身的大小。private heap indirection把 generic `F`
缩为pointer-sized future，正好切断这组重复内嵌。

同一 debug binary的只读 ARM64反汇编还显示以下 poll函数固定 stack allocation：

| poll函数 | native stack allocation |
| --- | ---: |
| target async test body | `0x6510 = 25,872 B` |
| `PreparedActivationRelativeServiceCall::wait` | `0x4890 = 18,576 B` |
| `await_actual_pending<activation wait>` | `0x14d80 = 85,376 B` |
| `eval_activation_relative_service_call` | `0x1ed10 = 126,224 B` |

这些 frame不能机械相加成一个虚构的精确峰值，但它们与 type-size层级和
`2.421875 -> 2.4375 MiB`翻转共同证明：debug poll链有有限且很大的栈搬运/保留，不是服务调用
或 catch语义递归。

## 5. 被排除的假设

| 假设 | 结论与证据 |
| --- | --- |
| full-suite并发或先行 test污染 | 排除。exact独立进程、默认 threads和`--test-threads=1`都失败。 |
| test把三个 case串行组合是唯一原因 | 排除。只有一个 linked-public import且无 catch的 `PUBLIC` 也在默认栈失败；组合只提高阈值。 |
| actual-Pending、deadline或Actor reacquire | 排除。第一个 provider body无悬挂点且first poll完成；fixture无Actor frame，有限高栈运行约0.01s完成。 |
| unlinked catch miss或InternalError import触发峰值 | 排除为本次触发路径。第一条 linked-public exact-catch await完成前已越过stack guard。 |
| service-error import/catch真实递归 | 排除。public import是有限的 exact link selection、decode、local carrier materialization；catch只做一次 linked identity match。 |
| fixture service循环 | 排除。`ConsumerTopology::OneHop`直接选择 terminal activation；terminal executable只Construct/Throw，没有service edge。 |
| evaluator无限递归 | 排除。`eval_program_expr_ref`、`eval_program_expr`、`call_program_executable_carriers`均由`async_recursion` boxing；fixture表达式深度有限，高栈立即完成。 |
| prepared service wait/finalize ownership错误 | 排除为本 blocker owner。J1已证明 `PreparedProviderUnary::wait` 是 owned `Future + Send + 'static`，不借 caller heap/env/Actor frame；`CompletedProviderUnary::finalize` 才重新取得 caller heap。 |
| 提高 `RUST_MIN_STACK`是修复 | 拒绝。它只定位阈值，会掩盖 production evaluator的debug stack放大。 |
| 修改test attribute、ignore或拆断言 | 拒绝。相邻单case production route同样失败，且真实 service caller会经过同一边界。 |

## 6. 唯一 owner、写集与语义冻结

唯一 owner：

```text
P5-F445H-E4R activation-relative service actual-Pending stack-safety closure
  -> EvalContext::eval_activation_relative_service_call
```

唯一允许的 production写集：

```text
runtime/eval/src/eval_context/actual_pending/activation.rs
```

冻结修复形状是在当前 line 49附近把同一个 future放入 private pinned heap：

```rust
let wait = Box::pin(operation.wait());
let completed = self.await_actual_pending(wait).await?;
```

然后仍调用同一个：

```rust
completed.finalize(self)
```

`PreparedActivationRelativeServiceCall::wait` 已是 `Future + Send + 'static`，所以该 indirection
不需要公共 type或 lifetime变化。它让 generic E3链只携带 pointer-sized future；prepare、
wait、finalize仍各自只发生一次。

明确禁止修改：

- `runtime/eval/src/assembly_execution/async_stream_cancel/activation_relative.rs`；
- `runtime/eval/src/assembly_execution/async_stream_cancel/prepared_unary.rs`；
- `runtime/eval/src/assembly_execution/service_error_channel.rs`；
- `runtime/eval/src/exceptions.rs`；
- 通用 `actual_pending.rs`、`ActorExecutionFrame`或program recursion owner；
- `service_error_consumer.rs`及其它 tests/fixtures；
- Cargo manifests、lockfile、stack/test runtime配置或公共 API。

必须保持的行为：

- activation-relative target/contract解析和provider参数prepare只执行一次；
- same owned unary wait只poll同一个provider invocation，drop不重建、不重放；
- first-Ready不释放Actor segment；真实 first-Pending仍由E3释放并在finalize前reacquire；
- provider error仍先由 `CompletedProviderUnary::finalize` 固化一次，再由caller import一次；
- linked public exact catch命中并保留caller-local value；
- unlinked public catch miss，fixed bytes与linked路径完全相同且保持opaque；
- private provider error仍固定为无私有payload的`std.service.InternalError`并被exact catch命中；
- same-service rethrow继续保留source、stack、correlation和raw fixed bytes；
- finalize仍发生在wait完成/Actor恢复之后，caller heap失败原子性和request owner生命周期不变。

若实现需要越出上述单文件、改变wait/finalize owner、catch identity或error import，E4R8 fix必须
停止并重新审视 scope；不能自行扩写公共 owner。

## 7. RED、focused GREEN 与一次完整 lib重验

所有 GREEN必须显式清除 `RUST_MIN_STACK`/`RUSTFLAGS`，不得设置自定义 test stack。

### 7.1 冻结 RED

```bash
env -u RUST_MIN_STACK -u RUSTFLAGS \
  CARGO_NET_OFFLINE=true CARGO_TARGET_DIR=$TARGET \
  cargo test -p skiff-runtime-eval --locked --lib "$EXACT" -- \
  --exact --nocapture

env -u RUST_MIN_STACK -u RUSTFLAGS \
  CARGO_NET_OFFLINE=true CARGO_TARGET_DIR=$TARGET \
  cargo test -p skiff-runtime-eval --locked --lib "$EXACT" -- \
  --exact --nocapture --test-threads=1
```

两者当前均为 stack overflow、exit `101`、`SIGABRT`。不得以改名、ignore、拆断言或提高栈
消除RED。

### 7.2 Focused GREEN

修复后依次要求：

```bash
# 上述两个 EXACT 命令：分别 1/1

env -u RUST_MIN_STACK -u RUSTFLAGS \
  CARGO_NET_OFFLINE=true CARGO_TARGET_DIR=$TARGET \
  cargo test -p skiff-runtime-eval --locked --lib "$PUBLIC" -- \
  --exact --nocapture --test-threads=1

env -u RUST_MIN_STACK -u RUSTFLAGS \
  CARGO_NET_OFFLINE=true CARGO_TARGET_DIR=$TARGET \
  cargo test -p skiff-runtime-eval --locked --lib \
  f445h_e4r_stream_activation_unary -- --nocapture --test-threads=1

env -u RUST_MIN_STACK -u RUSTFLAGS \
  CARGO_NET_OFFLINE=true CARGO_TARGET_DIR=$TARGET \
  cargo test -p skiff-runtime-eval --locked --lib \
  assembly_execution::ordinary::tests::service_error_consumer -- \
  --nocapture --test-threads=1

env -u RUST_MIN_STACK -u RUSTFLAGS \
  CARGO_NET_OFFLINE=true CARGO_TARGET_DIR=$TARGET \
  cargo test -p skiff-runtime-eval --locked --lib \
  f445h_e4r_spine -- --list

env -u RUST_MIN_STACK -u RUSTFLAGS \
  CARGO_NET_OFFLINE=true CARGO_TARGET_DIR=$TARGET \
  cargo test -p skiff-runtime-eval --locked --lib \
  f445h_e4r_spine -- --nocapture
```

验收分别为：

- `EXACT` 默认/单线程均 `1 passed / 0 failed / 394 filtered`；
- `PUBLIC` 为 `1/1`；
- activation unary Ready、Pending和actual provider failure import均保持GREEN；
- service-error consumer模块的5项全部通过，现有three-hop test自己的16 MiB thread不替代其它
  tests的默认栈；
- spine listing仍为23，execution为`23 passed / 0 failed`。

随后执行：

```bash
CARGO_NET_OFFLINE=true CARGO_TARGET_DIR=$TARGET \
  cargo check -p skiff-runtime-eval --tests --locked
CARGO_TARGET_DIR=$TARGET cargo fmt --check
git diff --check
```

### 7.3 一次完整 lib重验

E4R7 deadline owner与E4R8 stack owner合入同一个最终候选、focused GREEN后，由唯一完整 gate
owner运行一次：

```bash
env -u RUST_MIN_STACK -u RUSTFLAGS \
  CARGO_NET_OFFLINE=true CARGO_TARGET_DIR=$TARGET \
  cargo test -p skiff-runtime-eval --locked --lib -- \
  --nocapture --test-threads=1
```

必须取得合法：

```text
395 passed / 0 failed / 0 ignored / 0 filtered
```

不得用提高栈的对照、abort前零散`ok`行或推算汇总替代。E4R6记录的五条
`async_stream_cancel` deadline failures属于独立 E4R7 owner；若它们仍在最终候选出现，完整 lib
仍是FAIL，但不得扩张本 E4R8单文件写集。

## 8. 结论

原因、owner、单文件写集、保持语义和可执行验收均已冻结，故状态为
`READY_FOR_E4R8_FIX`，不需要用户设计决策。result提交前唯一 tracked写入必须是本文；提交后
worktree必须clean。
