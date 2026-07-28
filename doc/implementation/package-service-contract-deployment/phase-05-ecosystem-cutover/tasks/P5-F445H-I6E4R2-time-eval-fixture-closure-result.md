# P5-F445H-I6E4R2 Eval time fixture closure result

状态：

```text
COMPLETE
TASK_SCOPE_EXPANDED = NO
TASK_NOT_EXECUTABLE = NO
I6_TIME_COMPLETE = YES
```

Eval time projection fixture 的剩余墙钟竞态已闭合。测试不再把 current absolute deadline 固定为
测试起点后 5ms：current/outer deadline 分别留出 60s/120s 的首次真实 poll 余量；确认真实
`Pending` 与 `1 lease / 1 waiter / 1 timer` 后，fixture 以同一个 current scope 的 absolute
deadline owner 调用 `terminal_at(current_deadline)`。真实 native wait 随即返回内部 cancellation，
精确 owner 仍为 `LocalDeadlineExceeded`，terminal lifecycle 归零。

这只改变 test-only fixture 的时序控制；没有修改 production、time/scope 语义、公共契约、
native prepared fixture、Cargo/lockfile或兄弟 owner。

## 1. 候选身份

| 项 | commit / tree |
| --- | --- |
| baseline | `c0967ace89d1c835f7f53d498ea7f95a48beadbb` / `1e281e03880ee182693c5fc79fb7ab1ddfd9079d` |
| task + fixture implementation | `80a94bb886dbec35290aaf5ad4861fbab58d4b6b` / `3bb2bd4042e6d9b33bba31e52ddc360dfce247f3` |
| result publication | 本文件独立提交；精确 commit/tree 由最终交付消息记录，避免 commit 自引用 |

branch/worktree：

```text
codex/p5-f445h-i6e4r2-time-eval-fixture
/Users/geek/workspace/skiff-p5-f445h-i6e4r2-time-eval-fixture
```

## 2. RED 事实与本地复现口径

直接父结果
`P5-F445H-I6E4R-time-prepared-fixture-closure-result.md` 已在目标 fixture 最后一次生产提交
`0f250dff41ec91a06c89a4716b029d69e6edc116` 上连续两次记录真实 RED：

```text
cargo test -p skiff-runtime-eval \
  f445h_i6_time_projection_to_pending -- --nocapture

0 passed / 1 failed
runtime/eval/src/program_execution/execution_scope_tests.rs:1020
assertion failed: matches!(poll_time_pending(wait.as_mut()), Poll::Pending)
```

baseline 对目标文件的最后修改仍是上述 commit；baseline source 精确保留
`base + 5ms` current deadline，因此该 RED 对本节点输入有效。

本 Agent 在修改前的普通 selector 复跑为 `1/1`，随后直接执行同一未修改 test binary 的
128-way 并发压力复跑仍为 `128/128`。这没有推翻父节点 RED，而是确认缺陷属于调度相关的非稳定
fixture：同一测试可能在 5ms 内 poll 而通过，也可能在首次 poll 前越过 absolute deadline 而失败。
本结果不把这些本地 GREEN 伪记为 RED。

## 3. fixture 闭合证据

`runtime/eval/src/program_execution/execution_scope_tests.rs` 的目标测试现在：

1. 建立 current `base + 60s` 与 outer `base + 120s`，避免 projection/prepare 的墙钟开销抢先
   terminal；
2. 仍经过 `project_runtime_native_capability_context(Time)`、
   `NativeDispatch::prepare_resolved_native_call` 与
   `PreparedNativeCall::ExternalWait`；
3. 首次真实 poll 必须是 `Pending`，并继续断言 current scope lifecycle 为 `1 / 1 / 1`；
4. 在 clock 不需要推进的情况下，以 current absolute deadline owner 触发同一 local signal；
5. 第二次真实 poll 必须返回 native `RuntimeError::Cancelled`，scope 保留
   `LocalDeadlineExceeded`，lifecycle 为 `0 / 0 / 0`。

因此测试仍证明 projection 到真实 sleep pending owner，不退化为 getter test，也不通过等待 60s
掩盖问题。

## 4. 最终验证

| 层级 | 命令 | implementation tree 结果 | 覆盖 |
| --- | --- | --- | --- |
| Eval list | `cargo test -p skiff-runtime-eval f445h_i6_time_projection_to_pending -- --list` | PASS；`1 test` | selector 非零 |
| Eval run | `cargo test -p skiff-runtime-eval f445h_i6_time_projection_to_pending -- --nocapture` | PASS；`1/1` | projection、真实 Pending、deadline owner、terminal cleanup |
| native prepared | `cargo test -p skiff-runtime-native prepared_time_wait_does_not_borrow_caller_heap_and_observes_actual_pending -- --nocapture` | PASS；`1/1` | 既有 prepared-time heap/Pending 回归 |
| native list | `cargo test -p skiff-runtime-native f445h_i6_time_scope -- --list` | PASS；`7 tests` | selector 非零 |
| native run | `cargo test -p skiff-runtime-native f445h_i6_time_scope -- --nocapture` | PASS；`7/7` | normal/current/outer/ancestor/internal/zero/sync |
| compile | `cargo check -p skiff-runtime-native -p skiff-runtime-eval --locked` | PASS；仅既有 warnings | native + Eval locked 接线 |
| format | `cargo fmt --check` | PASS | Rust format |
| diff | `git diff --check` | PASS | whitespace |

没有运行 full gate、stable/live/network/MongoDB。

## 5. 实际写集与反向搜索

implementation commit 只包含：

```text
doc/implementation/package-service-contract-deployment/phase-05-ecosystem-cutover/tasks/P5-F445H-I6E4R2-time-eval-fixture-closure.md
runtime/eval/src/program_execution/execution_scope_tests.rs
```

result commit 另增加：

```text
doc/implementation/package-service-contract-deployment/phase-05-ecosystem-cutover/tasks/P5-F445H-I6E4R2-time-eval-fixture-closure-result.md
```

目标 Eval fixture 内反向搜索：

```text
rg 'from_millis\\(5\\)|sleep\\(Duration::from_millis\\(10\\)\\)' \
  runtime/eval/src/program_execution/execution_scope_tests.rs

0 hits
```

没有 production、公共契约、Cargo/lockfile、native prepared fixture或其它测试变化；没有
polling、fallback、全局/task-local side channel、merge、rebase或push。

## 6. 自验收矩阵

| 设计/任务条款 | 代码证据 | 反向搜索证据 | 测试 |
| --- | --- | --- | --- |
| 首次真实 poll 稳定 Pending | `execution_scope_tests.rs:989-1029` 的 60s/120s absolute deadline、真实 projection/prepare/poll 与 `1/1/1` | 目标文件无 `from_millis(5)` | Eval `1/1` |
| current deadline 保持精确 owner | `execution_scope_tests.rs:1031-1046` 以同一 `current_deadline` terminal，断言 cancellation、`LocalDeadlineExceeded`、idle | 无 wall-clock `sleep(10ms)` | Eval `1/1`；native `7/7` |
| prepared fixture 与 caller heap 不回归 | production/native fixture 零修改 | implementation 写集不含 `runtime/native` | prepared `1/1` |
| 不改变 production/time 契约 | 唯一 Rust 写入位于 `execution_scope_tests.rs` | baseline diff 无 production/Cargo/lockfile | locked check、fmt、diff PASS |

```text
I6_TIME_COMPLETE = YES
```
