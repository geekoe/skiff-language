# P5-F445H-E4R6 callback full-suite stack preflight result

状态：

```text
READY_FOR_E4R6_FIX
USER_DECISION_REQUIRED = NO
```

当前 blocker 已收敛为一个 production evaluator 的 stack-shape 问题，而不是 full-suite
并发碰撞：callback owner 的嵌套 evaluator future 被直接传入 generic
`await_actual_pending` 链，有限但很大的 async state在 poll调用链中重复放大。当前默认 test
worker stack落在失败侧；把 `RUST_MIN_STACK` 从 `2.03125 MiB` 提升到 `2.0625 MiB` 即由
`SIGABRT` 转为 `1/1` PASS。

最小修复节点是
`EvalContext::eval_callback_interface_call`：只在该 private production route为
`PreparedCallbackInvocation::wait` 引入 heap indirection，再进入 E3 actual-Pending owner。
不修改 callback prepared owner、test fixture、公共 API、Actor release/reacquire协议或
observable callback语义。

## 1. 候选身份与边界

| 项 | 值 |
| --- | --- |
| 诊断开始时 HEAD | `a5a4671c21fc637068030be53e05aed902e07e97` |
| 诊断开始时 tree | `f773c45b77c4f0c3228073c55eb625634190fca4` |
| 冻结 production/tests commit | `da49c17cb6e3c479ea649b936aab8614d3beface` |
| 冻结 production/tests tree | `0bdff47fad52aa52fea27bfd753db4bbf1213b6c` |
| worktree | `/Users/geek/workspace/skiff-p5-f445h-e4r6-preflight` |
| branch | `codex/p5-f445h-e4r6-preflight` |
| 独立 target | `/Users/geek/workspace/skiff-p5-f445h-e4r6-preflight/build/cargo-target` |

`da49c17c..a5a4671c` 只新增 E4R5 task/result与本 E4R6 task；production、tests、
Cargo manifests和 lockfile diff均为空。诊断开始时 `git status --short --branch` 只输出
branch header。环境中没有预设 `RUST_MIN_STACK`、`RUSTFLAGS`、Tokio test或 Cargo test
线程变量；机器有14个 logical CPUs，shell `ulimit -s` 为8176 KiB。后者不是 libtest所创建
worker thread的栈大小，不能解释或解除本 failure。

本任务没有修改 production/tests/fixture/manifest/lockfile，没有访问 network、stable
instance、MongoDB或其它仓库。临时日志只写入 ignored `build/e4r6-diagnostics/`，不提交。
三次完整 lib execution额度已全部使用。

## 2. 可复现命令与结果

以下名称在本节命令中展开为：

```bash
TARGET=/Users/geek/workspace/skiff-p5-f445h-e4r6-preflight/build/cargo-target
PENDING=actor_executor::tests::actor_concurrent_continuation::evaluator_actual_pending::callback_matrix::f445h_e4r_spine_callback_pending_reacquires_before_finalize
READY=actor_executor::tests::actor_concurrent_continuation::evaluator_actual_pending::callback_matrix::f445h_e4r_spine_callback_ready_keeps_actor_segment
```

### 2.1 精确 callback test与线程数

| # | 命令 | exit | 结果 |
| ---: | --- | ---: | --- |
| 1 | `CARGO_TARGET_DIR=$TARGET cargo test -p skiff-runtime-eval --locked --lib "$PENDING" -- --exact --nocapture` | `101` | `running 1 test`；stack overflow，`SIGABRT`。 |
| 2 | `CARGO_TARGET_DIR=$TARGET cargo test -p skiff-runtime-eval --locked --lib "$PENDING" -- --exact --nocapture --test-threads=1` | `101` | 同一 test单独串行仍 stack overflow，`SIGABRT`。 |
| 3 | `CARGO_TARGET_DIR=$TARGET cargo test -p skiff-runtime-eval --locked --lib "$READY" -- --exact --nocapture` | `101` | first-Ready sibling单独运行也 stack overflow，`SIGABRT`。 |

因此 E4R5 的“只在完整 suite中失败”不是当前候选上的真实条件。R1早期候选的 focused
`23/23` 是有效历史证据，但不能外推到经过 R2/R3/R4 后的冻结 production candidate。
当前 failure不依赖其它 test、执行顺序、并发或 outer Pending/reacquire分支。

### 2.2 栈阈值矩阵

以下均为精确 `PENDING` test、`--exact --nocapture`，只改变进程级
`RUST_MIN_STACK`；该变量仅用于定位，不是允许的修复。

| `RUST_MIN_STACK` | MiB | exit | 结果 |
| ---: | ---: | ---: | --- |
| 未设置 | 默认 | `101` | stack overflow / `SIGABRT` |
| `2129920` | `2.03125` | `101` | stack overflow / `SIGABRT` |
| `2162688` | `2.0625` | `0` | `1 passed / 394 filtered`，约0.03s |
| `2228224` | `2.125` | `0` | `1 passed / 394 filtered`，约0.03s |
| `2359296` | `2.25` | `0` | `1 passed / 394 filtered`，约0.03s |
| `2621440` | `2.5` | `0` | `1 passed / 394 filtered`，约0.03s |
| `3145728` | `3` | `0` | `1 passed / 394 filtered`，约0.03s |
| `4194304` | `4` | `0` | `1 passed / 394 filtered`，约0.03s |

阈值命令的精确形式为：

```bash
RUST_MIN_STACK=<表中值> CARGO_TARGET_DIR=$TARGET \
  cargo test -p skiff-runtime-eval --locked --lib "$PENDING" \
  -- --exact --nocapture
```

32 KiB宽的稳定翻转区间和通过后约0.03s完成共同排除无限递归；它们指向有限的 poll stack
peak。

### 2.3 三次完整 lib execution

| # | 命令 | inventory / exit | 结果 |
| ---: | --- | --- | --- |
| 1 | `CARGO_TARGET_DIR=$TARGET cargo test -p skiff-runtime-eval --locked --lib -- --nocapture --test-threads=1` | `395` / `101` | 打印18条 `ok` 后在 `PENDING` stack overflow并 `SIGABRT`；进程没有合法完整汇总。 |
| 2 | `CARGO_TARGET_DIR=$TARGET cargo test -p skiff-runtime-eval --locked --lib -- --nocapture --test-threads=2` | `395` / `101` | 同样打印18条 `ok` 后在 `PENDING` stack overflow并 `SIGABRT`；无合法完整汇总。 |
| 3 | `RUST_MIN_STACK=4194304 CARGO_TARGET_DIR=$TARGET cargo test -p skiff-runtime-eval --locked --lib -- --nocapture --test-threads=2` | `395` / `101` | callback Pending与Ready均通过且没有 stack overflow；最终 `390 passed / 5 failed`，五个 failure均是 `async_stream_cancel` deadline/timing test。 |

第三次执行的五个 failure为：

```text
assembly_execution::async_stream_cancel::tests::
  pending_provider_unary_wakes_from_deadline_and_cancels_provider_request
  provider_stream_deadline_terminal_reaches_pending_consumer_as_typed_timeout
  stream_item_deadline_remains_typed_through_provider_terminal
  stream_terminal_item_and_publication_deadlines_remain_typed
  terminal_publication_deadline_replaces_blocked_terminal_with_typed_timeout
```

这五项不在 callback调用链或本任务允许写集内，也不是 stack overflow；本只读 preflight
没有继续调查或把它们并入 E4R6 owner。它们是最终完整 gate的独立残余风险，不能把第三次
执行报告成 full GREEN。

## 3. Production调用链与 async state证据

静态调用链为：

```text
callback_matrix.rs
  -> EvalContext::exec_program_executable
  -> EvalContext::eval_program_call
  -> EvalContext::eval_program_interface_method_call
  -> EvalContext::eval_callback_interface_call
  -> assembly_execution::prepare_callback_capability_call
  -> PreparedCallbackInvocation::wait
  -> Interpreter::call_program_executable_with_self
  -> owner-local callback executable
  -> std.time.sleep(delay_ms)
  -> outer EvalContext::await_actual_pending
  -> actual_pending::await_operation
  -> ActorExecutionFrame::await_if_pending
  -> CompletedCallbackInvocation::finalize
```

使用现有源码、只编译不运行 test的 type-size诊断：

```bash
RUSTC_BOOTSTRAP=1 RUSTFLAGS=-Zprint-type-sizes CARGO_TARGET_DIR=$TARGET \
  cargo test -p skiff-runtime-eval --locked --lib --no-run
```

exit为 `0`。与该链直接相关的 debug async state为：

| async state | 大小 |
| --- | ---: |
| Pending callback `#[tokio::test]` future | `2,472 B` |
| Ready callback `#[tokio::test]` future | `1,048 B` |
| `PreparedCallbackInvocation::wait` | `5,104 B` |
| `ActorExecutionFrame::await_if_pending<callback wait>` | `11,456 B` |
| `actual_pending::await_operation<callback wait>` | `16,624 B` |
| `EvalContext::await_actual_pending<callback wait>` | `21,760 B` |
| `EvalContext::eval_callback_interface_call` | `21,976 B` |
| `EvalContext::eval_program_interface_method_call` | `22,392 B` |
| `EvalContext::eval_program_call` | `41,272 B` |
| `Interpreter::exec_program_executable` | `1,856 B` |
| `Interpreter::call_program_executable_with_self` | `1,600 B` |

`EvalContext::exec_program_executable` 本身因递归boxing只有 `32 B`，但 callback route会在
外层 evaluator future中同步 poll一次 owner-local evaluator；未boxing的5,104-byte callback
wait又作为 generic `F` 被依次嵌入11,456、16,624和21,760-byte state。测试 future自身只有
1–2.5 KiB，故问题不在 libtest harness或fixture payload；stack amplification发生在
production evaluator的 callback actual-Pending边界。

当前共同 `eval_program_call` future已达41,272 B；结合 R1之后新增的 evaluator分支，这可
解释为何旧 R1 focused证据仍真实、当前精确 test却越过默认 stack余量。本任务没有测量旧
候选的 type size，也没有对历史提交做运行时二分，因此不把某一个后续 commit或精确增长量
虚构为唯一 regression归因；唯一运行时原因不依赖该历史归因。

LLDB曾用下列命令在4 MiB对照进程中设置符号断点：

```text
lldb build/cargo-target/debug/deps/skiff_runtime_eval-d47a3a59d9e65bfd
(lldb) settings set target.env-vars RUST_MIN_STACK=4194304
(lldb) breakpoint set -r "PreparedCallbackInvocation.*wait.*closure"
(lldb) run <PENDING> --exact --nocapture
```

该进程没有取得可用 frame trace，随后人工终止，因而没有可报告的 test summary或 exit。
本文不据此声称某一个具体机器指令frame溢出；定位依据是精确 test隔离、线程数矩阵、
32 KiB栈阈值、Ready/Pending对照、async state尺寸和静态调用链。

## 4. 被排除的假设

| 假设 | 结论与证据 |
| --- | --- |
| full-suite并发 | 排除。精确 test单独失败；`--test-threads=1`与`2`均失败。 |
| 先行 test污染/全局fixture碰撞 | 排除。独立进程的单个 exact test失败；fixture每次新建 runtime assembly、activation、request carrier和 local adapter。 |
| Pending/reacquire或timeout test结构独有 | 排除。没有 outer timeout的 first-Ready sibling也独立失败。 |
| callback owner真实无限递归 | 排除。fixture owner IR只执行一次 `std.time.sleep(delay_ms)`并返回 `"callback-complete"`，没有 callback edge；`PreparedCallbackInvocation::wait`只调用一次 owner-local executable；阈值以上约0.03s完成。 |
| callback request generation或全局 adapter registry | 排除。carrier按 fixture的 activation/request generation本地注册；该路径使用 `InProcessCallbackAdapter::from_local_interface`，不经过 native explicit-adapter global registry。 |
| prepared callback ownership协议错误 | 排除为本 stack blocker的owner。prepare已把caller值 detach到 owner heap，owned context明确拒绝 caller Actor frame，wait只保留 owner guard，finalize在reacquire后单次导入。问题是 wait future进入 generic evaluator链时的栈布局。 |
| 单纯增大 `RUST_MIN_STACK`是修复 | 拒绝。它只证明stack threshold，会掩盖 production route的debug stack放大，不能成为代码或CI合同。 |
| test-only拆分/隔离可修 | 排除。exact test已经是最小独立进程复现，故只改 test结构不会解除真实 route的stack风险。 |

代码中唯一与该测试目录有关的 static是其它 canonical-emit fixture的
`NEXT_ID: AtomicU64`，不在 callback route。native模块虽有全局
`EXPLICIT_NATIVE_ADAPTERS`，本 fixture不使用它。

## 5. 冻结修复 owner、写集与语义边界

唯一 owner：

```text
P5-F445H-E4R1 callback actual-Pending stack-safety closure
  -> EvalContext::eval_callback_interface_call
```

唯一允许的 production写集：

```text
runtime/eval/src/eval_context/actual_pending.rs
```

可执行修复形状是在 `eval_callback_interface_call` 内，将
`prepared.wait(&interpreter)` 在传给 `await_actual_pending` 前作 private heap
indirection（例如 call-site `Box::pin`），让 E3 generic链携带 pointer-sized future，而不是
逐层内嵌5,104-byte concrete wait state。boxing必须仍由相同
`eval_callback_interface_call` await并在完成后调用相同 `finalize(self.heap)`。

明确不在写集：

- `runtime/eval/src/assembly_execution/callback_native/prepared.rs`；
- `callback_matrix.rs`或其它 tests/fixtures；
- `ActorExecutionFrame`、通用 `await_actual_pending`、Tokio/libtest配置；
- Cargo manifests、lockfile、公共 API或callback capability owner。

这是 production内部内存布局修复，不是 test隔离；它不改变 callback prepared owner、
执行次数、错误传播、Actor first-Ready/first-Pending判断、release/reacquire时序、
caller-heap finalize顺序或公共语义。若实现证明必须越出上述单文件，修复任务必须停下并重新
审视 scope，不能自行扩写公共 owner。

## 6. 修复任务 RED/GREEN

所有 GREEN命令必须在没有 `RUST_MIN_STACK`、没有自定义 test stack和同一独立 target下运行。

### RED

冻结候选已有两个无需新增test的真实 RED：

```bash
CARGO_TARGET_DIR=$TARGET cargo test -p skiff-runtime-eval --locked --lib \
  "$PENDING" -- --exact --nocapture

CARGO_TARGET_DIR=$TARGET cargo test -p skiff-runtime-eval --locked --lib \
  "$READY" -- --exact --nocapture
```

两者当前均为 `running 1 test` 后 stack overflow、exit `101`、`SIGABRT`。修复提交不得通过
删除、ignore、改名或提高栈来消除 RED。

### Focused GREEN

实现后先要求：

```bash
CARGO_TARGET_DIR=$TARGET cargo test -p skiff-runtime-eval --locked --lib \
  "$PENDING" -- --exact --nocapture

CARGO_TARGET_DIR=$TARGET cargo test -p skiff-runtime-eval --locked --lib \
  "$READY" -- --exact --nocapture

CARGO_TARGET_DIR=$TARGET cargo test -p skiff-runtime-eval --locked --lib \
  f445h_e4r_spine -- --list

CARGO_TARGET_DIR=$TARGET cargo test -p skiff-runtime-eval --locked --lib \
  f445h_e4r_spine -- --nocapture
```

验收分别为 `1/1`、`1/1`、精确 listing `23`、execution `23 passed / 0 failed`，且没有
stack overflow。

### 一次完整 lib重验

只在 focused GREEN后执行一次默认栈串行重验：

```bash
CARGO_TARGET_DIR=$TARGET cargo test -p skiff-runtime-eval --locked --lib \
  -- --nocapture --test-threads=1
```

必须得到合法 `395 passed / 0 failed / 0 ignored / 0 filtered` summary；不得用4 MiB对照或
abort前的零散 `ok` 行替代。随后执行：

```bash
CARGO_TARGET_DIR=$TARGET cargo check -p skiff-runtime-eval --tests --locked
CARGO_TARGET_DIR=$TARGET cargo fmt --check
git diff --check
```

并静态确认 callback prepared owner、Actor lease/reacquire和finalize断言未被弱化。

## 7. 未决项与残余风险

- callback stack blocker已有单一原因、唯一owner、单文件写集和可执行验收，不需要用户设计
  决策，故状态为 `READY_FOR_E4R6_FIX`。
- 没有可用 overflow backtrace，不能把溢出归到一个具体机器frame；现有控制变量证据足以
  冻结安全的 heap-boundary修复节点。
- 4 MiB / 两线程完整对照暴露的五个 `async_stream_cancel` deadline failures尚未归因。它们
  不扩张本 callback修复写集，但可能阻止修复后的完整 `395/395` gate；完整重验若仍出现，
  必须作为独立 blocker记录，不能塞入 E4R6 callback patch。
- result提交前，tracked diff必须只包含本文；提交后 worktree必须恢复 clean。本任务不
  merge、rebase或push。
