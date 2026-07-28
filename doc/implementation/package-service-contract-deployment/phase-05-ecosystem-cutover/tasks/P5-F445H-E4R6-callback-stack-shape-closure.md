# P5-F445H-E4R6 callback stack-shape closure

状态：Ready。修复 E4R5 完整 gate 中 callback actual-Pending 的默认线程栈 blocker。完成后仍需
新的 R5C 独立完整验收；本节点不直接签发 E4R完成。

## 直接父节点与冻结根因

- `P5-F445H-E4R6-callback-full-suite-stack-preflight-result.md`
- `P5-F445H-E4R5-combined-integration-acceptance-result.md`
- `P5-F445H-E4R1-evaluator-spine-actual-pending-checkpoint-result.md`

production/tests仍冻结为
`da49c17cb6e3c479ea649b936aab8614d3beface`；后续 commits只增加 task/result。

E4R6 preflight已排除 test并发、全局fixture污染、真实无限递归和test-only隔离问题。当前
`PreparedCallbackInvocation::wait` 的约 5 KiB concrete async state被直接嵌入通用
actual-Pending / E3 future链，callback route poll stack峰值落在默认worker栈失败侧：

- 2.03125 MiB：失败；
- 2.0625 MiB：通过；
- Ready与Pending精确test在默认栈均 `SIGABRT`；
- 提高 `RUST_MIN_STACK`只用于定位，不是允许的修复。

唯一owner为 `EvalContext::eval_callback_interface_call`。

## 唯一写集

Production：

- `runtime/eval/src/eval_context/actual_pending.rs`

交付文档：

- 新增 `P5-F445H-E4R6-callback-stack-shape-closure-result.md`

不得修改：

- callback prepared owner
  `runtime/eval/src/assembly_execution/callback_native/prepared.rs`；
- callback或其它tests/fixtures；
-通用 `ActorExecutionFrame` / `await_actual_pending`；
-Cargo/manifest/lockfile、公共 API或其它production/result。

## 精确实现

只在 `EvalContext::eval_callback_interface_call` 内，把
`prepared.wait(&interpreter)` 在传给 `await_actual_pending` 前放到heap上的 private pinned
box中，使generic E3链只携带pointer-sized future，而不是逐层内嵌callback wait state。

必须保持：

- 同一个 prepared operation只 wait一次、finalize一次；
- first-Ready不释放 Actor segment；
- first-Pending才释放，完成后先reacquire/fence，再在caller heap finalize；
- error、drop、request generation和owner guard语义不变；
-不修改 callback capability owner、公共类型或通用actual-Pending路径；
-不通过增大线程栈、改test attribute、ignore/删test或降低断言绕过。

若一个call-site `Box::pin`不能闭合，或正确修复需要修改唯一写集外代码，立即返回
`TASK_SCOPE_EXPANDED`；不得扩大到公共 owner。

## 已冻结 RED 与验证

不新增测试。以下两个 exact tests在当前候选、默认环境中均为真实 RED：

```text
actor_executor::tests::actor_concurrent_continuation::
evaluator_actual_pending::callback_matrix::
f445h_e4r_spine_callback_pending_reacquires_before_finalize

actor_executor::tests::actor_concurrent_continuation::
evaluator_actual_pending::callback_matrix::
f445h_e4r_spine_callback_ready_keeps_actor_segment
```

使用独立 target，所有命令必须确保 `RUST_MIN_STACK` 未设置：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4r6-fix/build/cargo-target \
  cargo test -p skiff-runtime-eval --locked --lib \
  f445h_e4r_spine_callback_pending_reacquires_before_finalize -- --exact --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4r6-fix/build/cargo-target \
  cargo test -p skiff-runtime-eval --locked --lib \
  f445h_e4r_spine_callback_ready_keeps_actor_segment -- --exact --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4r6-fix/build/cargo-target \
  cargo test -p skiff-runtime-eval --locked --lib f445h_e4r_spine -- --list
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4r6-fix/build/cargo-target \
  cargo test -p skiff-runtime-eval --locked --lib f445h_e4r_spine -- --nocapture
```

必须分别为1/1、1/1、23 listed、23/23，且无stack overflow。focused GREEN后只运行一次默认栈
串行完整 lib：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4r6-fix/build/cargo-target \
  cargo test -p skiff-runtime-eval --locked --lib -- --nocapture --test-threads=1
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4r6-fix/build/cargo-target \
  cargo check -p skiff-runtime-eval --tests --locked
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4r6-fix/build/cargo-target \
  cargo fmt --check
git diff --check
```

若完整 lib为395/395，本节点状态可为 `CALLBACK_STACK_FIXED / LIB_GREEN`。若 callback已通过但
preflight观察到的五条 `async_stream_cancel` deadline tests失败，保留本 scoped callback提交并
返回 `CALLBACK_STACK_FIXED / FULL_LIB_BLOCKED_BY_DEADLINE_OWNER`，精确记录失败；不得修改stream
owner。任何其它失败同样按唯一owner分类，不得吞进本任务。

不运行combined、完整含integration/doc的eval gate、stable、live、network或 MongoDB。

result必须记录 implementation/result commit、实际diff、默认环境证明、两个exact与23-test
结果、完整lib汇总或新blocker、check/fmt/diff、未决问题和clean状态。

## Worktree 与交付

```text
worktree /Users/geek/workspace/skiff-p5-f445h-e4r6-fix
branch   codex/p5-f445h-e4r6-fix
```

不得派子 Agent。先提交production fix，再单独提交result；返回两个 commit、状态、测试计数、
新blocker（若有）和clean worktree。不得 merge、rebase或 push。

风险：高。此节点只关闭callback stack shape，不得替代最终独立验收。
