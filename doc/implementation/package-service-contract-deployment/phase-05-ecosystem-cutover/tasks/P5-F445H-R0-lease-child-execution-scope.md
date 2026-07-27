# P5-F445H-R0 Lease child execution scope

状态：Ready。F445H DAG 的 I4 correction。

## 直接父节点

- `P5-F445F-scoped-execution-control-checkpoint-result.md`
- `P5-F445H-eval-concurrency-owner-preflight-result.md`

## 完成目标

`ExecutionScopeLease` 新增：

```rust
pub fn child_execution_scope(&self) -> ExecutionScope
```

返回的 child scope 必须：

1. 保留 lease 所属 scope 的 effective deadline、deadline source、nesting、local cancellation 与
   shared lifecycle；
2. 把该 lease 的 `child_cancellation_token()` 追加为 ancestor cancellation signal；
3. lease drop、ancestor terminal、local deadline terminal、inherited deadline terminal结算后，
   child scope 观察 `AncestorCancelled`；
4. parent scope不因 lease child cancel受到反向污染；
5. `completion.complete()` normal success 不取消 child scope，也不制造虚假 terminal；
6. active lease/waiter/timer accounting继续精确归零。

不得公开任意“从 token 拼 scope”的构造器，不改变现有 deadline tie-break、terminal priority 或
request accounting。现有 `child_cancellation_token()` 可保留供底层 wait 使用，但 I5 lane
current context 必须能消费完整 child scope。

## Test-first 与验收

先在 `scoped_execution_tests.rs` 新增 RED，至少覆盖：

- child effective deadline/source/nesting与 lease scope相同；
- normal completion 后 child/parent 均无 cancel，lifecycle归零；
- lease drop 只让 child scope `AncestorCancelled`，parent仍正常；
- request/ancestor cancel settle 后 child scope cancelled；
- owned local deadline settle 后 child scope cancelled，parent terminal仍保持原 local owner；
- inherited deadline settle 后 child scope cancelled，parent/outer owner分类不变；
- cancel 与 deadline同 ready仍保持现有 cancel-first；
- 多 lease child token互不污染，任一 child cancel不取消 sibling或parent。

使用 Tokio paused clock，不用 wall-clock sleep。

运行：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-r0-lease-child-scope/build/cargo-target \
  cargo test -p skiff-runtime-capability-context scoped_execution -- --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-r0-lease-child-scope/build/cargo-target \
  cargo test -p skiff-runtime-capability-context --no-fail-fast
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-r0-lease-child-scope/build/cargo-target \
  cargo check -p skiff-runtime-eval --locked
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-r0-lease-child-scope/build/cargo-target \
  cargo fmt --check
git diff --check
```

eval check 仍可能只因 F445G 新 IR exhaustive arms而 RED；若如此，必须确认没有新增
capability-context API/typing错误并精确记录，不能把 I5 未实现伪报为本节点失败。

## 写集与提交

只允许：

- `runtime/capability-context/src/scoped_execution.rs`
- `runtime/capability-context/src/scoped_execution/lease.rs`
- `runtime/capability-context/src/scoped_execution_tests.rs`
- 本 result

不得修改 request、eval、host、native、artifact、compiler、Router 或其它 fixture。

worktree：

`/Users/geek/workspace/skiff-p5-f445h-r0-lease-child-scope`

branch：

`codex/p5-f445h-r0-lease-child-scope`

base：`b60f2d44`，再 cherry-pick 本任务文档。

提交 implementation，再只新增并提交：

`P5-F445H-R0-lease-child-execution-scope-result.md`

最终 clean。不得派子 Agent、merge/rebase/push、stable/live/network。若需要扩大字段/public API或
修改 request owner，停止并上报。
