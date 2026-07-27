# P5-F445H-R0 Lease child execution scope result

状态：`IMPLEMENTATION_COMPLETE / CAPABILITY_CONTEXT_GREEN / EXPECTED_I5_EVAL_RED`。

`ExecutionScopeLease::child_execution_scope()` 已提供完整 lane child current scope。它保留 lease
所属 scope 的 effective deadline、deadline source、nesting、local cancellation 和 shared
lifecycle，只把该 lease 的 child cancellation token 追加为 ancestor signal。没有公开任意 token
拼装 scope 的构造器，也没有修改 request、eval、host、native、artifact、compiler 或 Router。

## 1. 输入、写集与提交

| 项 | commit |
| --- | --- |
| production base | `b60f2d44` |
| task document | `f426fd34` |
| implementation | `77be0360` |

implementation 写集精确为：

- `runtime/capability-context/src/scoped_execution.rs`
- `runtime/capability-context/src/scoped_execution/lease.rs`
- `runtime/capability-context/src/scoped_execution_tests.rs`

`ExecutionScope::with_lease_child_cancellation` 是 module-private helper；public surface 只新增任务指定
的 lease method。child scope clone 共享原 local cancellation source 与 lifecycle，不创建新的
deadline owner、timer或 request accounting。

## 2. Test-first 证据

先只修改 `scoped_execution_tests.rs`，再运行 focused command，exit `101`。唯一 RED 是 7 个
`E0599`，全部精确报告 `ExecutionScopeLease` 缺少 `child_execution_scope`；随后才修改
production。

新增 paused-clock coverage 锁定：

- child effective deadline、source、nesting、lifecycle 与 lease scope 相同；
- normal completion 后 child 与 parent 都不取消，lifecycle 归零；
- lease drop 只让该 child 观察 `AncestorCancelled`；
- request/ancestor cancel settle 后 child 观察 `AncestorCancelled`；
- local deadline 的 lease settle 后 child 观察 ancestor cancel，parent 仍是原
  `LocalDeadlineExceeded` owner；
- inherited deadline 的 lease settle 后 child 观察 ancestor cancel，原 scope/outer scope
  继续分别保持 inherited/local owner；
- cancel 与 deadline 同 ready 时仍为 cancel-first；
- 两个 lease 的 child cancellation 隔离，取消一个不污染 sibling 或 parent；
- active lease、waiter、timer 在 normal、drop、cancel、local/inherited deadline 后归零。

所有异步测试使用 `#[tokio::test(start_paused = true)]` 与 `tokio::time::advance`，没有 wall-clock
sleep。

## 3. 验证

| 命令 | 结果 |
| --- | --- |
| `cargo test -p skiff-runtime-capability-context scoped_execution -- --nocapture` | PASS：9/9 |
| `cargo test -p skiff-runtime-capability-context --no-fail-fast` | PASS：52 unit tests、2 doc-tests |
| `cargo fmt --check` | PASS |
| `git diff --check` | PASS |
| `cargo check -p skiff-runtime-eval --locked` | 预期 RED：只有 2 个 I5 exhaustive match error |

所有 Cargo 命令使用独立 target：

```text
/Users/geek/workspace/skiff-p5-f445h-r0-lease-child-scope/build/cargo-target
```

eval check 的两个 `E0004` 与 F445H preflight 完全一致：

- `eval_context.rs::exec_program_statement` 尚未覆盖
  `LinkedStmtIr::{Timeout, Concurrent}`；
- `eval_context.rs::eval_program_expr` 尚未覆盖
  `LinkedExprIr::{Timeout, ConcurrentValue}`。

没有 capability-context API、typing 或本节点新增错误。一个既有 unreachable-pattern warning 与
linker dead-code warnings 不属于本任务写集。

## 4. 后继合同

I5 lane current context 应直接安装 `lease.child_execution_scope()` 对应的 owned control。lease
drop 或任一 control terminal settle 后，嵌套 host/native/stream invocation 从完整 current scope
观察结构化 `AncestorCancelled`；normal completion 不产生虚假 terminal。parent 和 sibling
execution scope 不受 child cancellation 反向污染。
